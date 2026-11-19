// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

package main

import (
	"bytes"
	"crypto/tls"
	"encoding/hex"
	"encoding/json"
	"errors"
	"io"
	"net"
	"sync"
	"time"

	"golang.org/x/net/http2"
	"golang.org/x/net/http2/hpack"
)

type rawStep struct {
	Hex    string `json:"hex"`
	Repeat int    `json:"repeat"`
}

type rawArgs struct {
	dialArgs
	ALPN      []string  `json:"alpn"`
	Preface   *bool     `json:"preface"`
	Steps     []rawStep `json:"steps"`
	ReadMS    int       `json:"read_ms"`
	MaxFrames int       `json:"max_frames"`
}

type rawFrame struct {
	Type      string      `json:"type"`
	Flags     uint8       `json:"flags"`
	Stream    uint32      `json:"stream"`
	Length    uint32      `json:"length"`
	ErrCode   string      `json:"error_code,omitempty"`
	Debug     string      `json:"debug,omitempty"`
	LastID    uint32      `json:"last_stream_id,omitempty"`
	Increment uint32      `json:"increment,omitempty"`
	Settings  [][2]uint32 `json:"settings,omitempty"`
	Headers   [][2]string `json:"headers,omitempty"`
	Ack       bool        `json:"ack,omitempty"`
	EndStream bool        `json:"end_stream,omitempty"`
	DataHex   string      `json:"data_hex,omitempty"`
}

func rawDial(a *dialArgs, alpn []string) (*tls.Conn, error) {
	cfg, err := clientTLS(a, alpn)
	if err != nil {
		return nil, err
	}
	d := &net.Dialer{Timeout: timeout(a.TimeoutMS)}
	conn, err := tls.DialWithDialer(d, "tcp", a.Addr, cfg)
	if err != nil {
		return nil, err
	}
	return conn, nil
}

func describe(f http2.Frame) rawFrame {
	h := f.Header()
	r := rawFrame{Type: h.Type.String(), Flags: uint8(h.Flags), Stream: h.StreamID, Length: h.Length}
	switch f := f.(type) {
	case *http2.SettingsFrame:
		r.Ack = f.IsAck()
		_ = f.ForeachSetting(func(s http2.Setting) error {
			r.Settings = append(r.Settings, [2]uint32{uint32(s.ID), s.Val})
			return nil
		})
	case *http2.PingFrame:
		r.Ack = f.IsAck()
		r.DataHex = hex.EncodeToString(f.Data[:])
	case *http2.GoAwayFrame:
		r.ErrCode = f.ErrCode.String()
		r.Debug = string(f.DebugData())
		r.LastID = f.LastStreamID
	case *http2.RSTStreamFrame:
		r.ErrCode = f.ErrCode.String()
	case *http2.WindowUpdateFrame:
		r.Increment = f.Increment
	case *http2.MetaHeadersFrame:
		r.Type = "HEADERS"
		r.EndStream = f.StreamEnded()
		for _, hf := range f.Fields {
			r.Headers = append(r.Headers, [2]string{hf.Name, hf.Value})
		}
	case *http2.DataFrame:
		r.EndStream = f.StreamEnded()
		d := f.Data()
		if len(d) > 64 {
			d = d[:64]
		}
		r.DataHex = hex.EncodeToString(d)
	}
	return r
}

// readFrames reads until EOF, an error, the deadline or max frames.
func readFrames(conn net.Conn, fr *http2.Framer, readMS, maxFrames int) ([]rawFrame, string) {
	if readMS <= 0 {
		readMS = 2000
	}
	if maxFrames <= 0 {
		maxFrames = 10000
	}
	_ = conn.SetReadDeadline(time.Now().Add(time.Duration(readMS) * time.Millisecond))
	var frames []rawFrame
	for len(frames) < maxFrames {
		f, err := fr.ReadFrame()
		if err != nil {
			var ne net.Error
			switch {
			case errors.Is(err, io.EOF):
				return frames, "eof"
			case errors.As(err, &ne) && ne.Timeout():
				return frames, "timeout"
			default:
				return frames, "error: " + err.Error()
			}
		}
		frames = append(frames, describe(f))
	}
	return frames, "max_frames"
}

func cmdRaw(raw json.RawMessage) (any, error) {
	var a rawArgs
	if err := json.Unmarshal(raw, &a); err != nil {
		return nil, err
	}
	alpn := a.ALPN
	if alpn == nil {
		alpn = []string{"h2"}
	}
	conn, err := rawDial(&a.dialArgs, alpn)
	if err != nil {
		return map[string]any{"dial_error": err.Error()}, nil
	}
	defer conn.Close()
	st := conn.ConnectionState()
	var out bytes.Buffer
	if a.Preface == nil || *a.Preface {
		out.WriteString(http2.ClientPreface)
	}
	for _, s := range a.Steps {
		b, err := hex.DecodeString(s.Hex)
		if err != nil {
			return nil, err
		}
		n := s.Repeat
		if n <= 0 {
			n = 1
		}
		for i := 0; i < n; i++ {
			out.Write(b)
		}
	}
	// Write in a goroutine: a server that stops reading must not stop the
	// frames it already sent from being read here.
	writeErr := make(chan error, 1)
	go func() {
		_ = conn.SetWriteDeadline(time.Now().Add(10 * time.Second))
		_, err := conn.Write(out.Bytes())
		writeErr <- err
	}()
	fr := http2.NewFramer(io.Discard, conn)
	fr.SetMaxReadFrameSize(1 << 24)
	fr.ReadMetaHeaders = hpack.NewDecoder(4096, nil)
	fr.MaxHeaderListSize = 16 << 20
	frames, end := readFrames(conn, fr, a.ReadMS, a.MaxFrames)
	result := map[string]any{
		"alpn":   st.NegotiatedProtocol,
		"frames": frames,
		"end":    end,
	}
	select {
	case err := <-writeErr:
		if err != nil {
			result["write_error"] = err.Error()
		}
	case <-time.After(100 * time.Millisecond):
		result["write_error"] = "write still blocked"
	}
	return result, nil
}

type burstArgs struct {
	dialArgs
	Streams int    `json:"streams"`
	Path    string `json:"path"`
}

// cmdBurst opens many concurrent unary streams on one connection, written
// back to back, and counts how each ends. It keeps the connection's receive
// window open and acknowledges SETTINGS and PING.
func cmdBurst(raw json.RawMessage) (any, error) {
	var a burstArgs
	if err := json.Unmarshal(raw, &a); err != nil {
		return nil, err
	}
	if a.Path == "" {
		a.Path = "/walletrpc.VersionService/Version"
	}
	conn, err := rawDial(&a.dialArgs, []string{"h2"})
	if err != nil {
		return map[string]any{"dial_error": err.Error()}, nil
	}
	defer conn.Close()
	var wmu sync.Mutex
	fr := http2.NewFramer(conn, conn)
	fr.ReadMetaHeaders = hpack.NewDecoder(4096, nil)
	fr.MaxHeaderListSize = 16 << 20
	if _, err := conn.Write([]byte(http2.ClientPreface)); err != nil {
		return nil, err
	}
	if err := fr.WriteSettings(); err != nil {
		return nil, err
	}
	// Open the connection window wide, as grpc-go's client does once its
	// BDP estimator has run.
	if err := fr.WriteWindowUpdate(0, 1<<30); err != nil {
		return nil, err
	}

	var hbuf bytes.Buffer
	enc := hpack.NewEncoder(&hbuf)
	start := time.Now()
	go func() {
		for i := 0; i < a.Streams; i++ {
			id := uint32(2*i + 1)
			wmu.Lock()
			hbuf.Reset()
			for _, f := range [][2]string{
				{":method", "POST"}, {":scheme", "https"}, {":path", a.Path},
				{":authority", a.Addr}, {"content-type", "application/grpc"},
				{"te", "trailers"},
			} {
				_ = enc.WriteField(hpack.HeaderField{Name: f[0], Value: f[1]})
			}
			_ = fr.WriteHeaders(http2.HeadersFrameParam{StreamID: id, BlockFragment: hbuf.Bytes(), EndHeaders: true})
			_ = fr.WriteData(id, true, []byte{0, 0, 0, 0, 0})
			wmu.Unlock()
		}
	}()

	statuses := map[string]int{}
	rst := map[string]int{}
	var goaway *rawFrame
	done := 0
	end := ""
	_ = conn.SetReadDeadline(time.Now().Add(timeout(a.TimeoutMS)))
	for done < a.Streams {
		f, err := fr.ReadFrame()
		if err != nil {
			end = err.Error()
			break
		}
		switch f := f.(type) {
		case *http2.SettingsFrame:
			if !f.IsAck() {
				wmu.Lock()
				_ = fr.WriteSettingsAck()
				wmu.Unlock()
			}
		case *http2.PingFrame:
			if !f.IsAck() {
				wmu.Lock()
				_ = fr.WritePing(true, f.Data)
				wmu.Unlock()
			}
		case *http2.MetaHeadersFrame:
			if f.StreamEnded() {
				code := "missing"
				for _, hf := range f.Fields {
					if hf.Name == "grpc-status" {
						code = hf.Value
					}
				}
				statuses[code]++
				done++
			}
		case *http2.RSTStreamFrame:
			rst[f.ErrCode.String()]++
			if f.ErrCode != http2.ErrCodeNo {
				done++
			}
		case *http2.GoAwayFrame:
			d := describe(f)
			goaway = &d
		}
	}
	elapsed := time.Since(start)
	return map[string]any{
		"completed":  done,
		"statuses":   statuses,
		"rst":        rst,
		"goaway":     goaway,
		"end":        end,
		"elapsed_ms": float64(elapsed.Microseconds()) / 1000,
	}, nil
}
