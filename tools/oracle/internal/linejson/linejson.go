// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

// Package linejson is the protocol every oracle binary speaks: one JSON
// object per line on stdin, one per line on stdout, in order.
//
// A request is {"cmd": "<name>", "args": <any JSON>}. The response is
// {"result": <any JSON>} on success or {"error": "<text>"} on failure; a
// handler may return an *Error to add a "kind" member. A request that panics
// ends the process, so that a Go panic is seen as one by the Rust side.
package linejson

import (
	"bufio"
	"encoding/json"
	"fmt"
	"os"
)

// Handler serves one command. args is the request's raw "args" member, or
// nil when the request has none.
type Handler func(args json.RawMessage) (any, error)

// Error is an error with a machine-readable kind.
type Error struct {
	Kind string
	Text string
}

func (e *Error) Error() string { return e.Text }

type request struct {
	Cmd  string          `json:"cmd"`
	Args json.RawMessage `json:"args"`
}

type response struct {
	Result any    `json:"result,omitempty"`
	Error  string `json:"error,omitempty"`
	Kind   string `json:"kind,omitempty"`
	// Failed distinguishes an error whose text is empty from a success.
	Failed bool `json:"failed,omitempty"`
}

// maxLine bounds one request line.
const maxLine = 256 << 20

// Serve reads requests from stdin until EOF and answers each on stdout.
func Serve(handlers map[string]Handler) {
	in := bufio.NewReaderSize(os.Stdin, 1<<20)
	out := bufio.NewWriterSize(os.Stdout, 1<<20)
	defer out.Flush()
	enc := json.NewEncoder(out)
	enc.SetEscapeHTML(false)
	for {
		line, err := readLine(in)
		if err != nil {
			return
		}
		var req request
		var resp response
		if err := json.Unmarshal(line, &req); err != nil {
			resp = response{Error: "bad request: " + err.Error(), Failed: true}
		} else if h, ok := handlers[req.Cmd]; !ok {
			resp = response{Error: fmt.Sprintf("unknown command %q", req.Cmd), Failed: true}
		} else {
			result, err := h(req.Args)
			if err != nil {
				resp = response{Error: err.Error(), Failed: true}
				if e, ok := err.(*Error); ok {
					resp.Kind = e.Kind
				}
			} else {
				resp = response{Result: result}
			}
		}
		if err := enc.Encode(&resp); err != nil {
			fmt.Fprintln(os.Stderr, "oracle: encode:", err)
			os.Exit(1)
		}
		if in.Buffered() == 0 {
			out.Flush()
		}
	}
}

func readLine(in *bufio.Reader) ([]byte, error) {
	var line []byte
	for {
		chunk, err := in.ReadSlice('\n')
		line = append(line, chunk...)
		if err == bufio.ErrBufferFull {
			if len(line) > maxLine {
				return nil, fmt.Errorf("request line too long")
			}
			continue
		}
		if err != nil && len(line) == 0 {
			return nil, err
		}
		return line, nil
	}
}
