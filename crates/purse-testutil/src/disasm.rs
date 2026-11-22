// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

//! A structural check of compiled code that must not branch on, or index
//! memory by, a secret: the running test executable's own machine code for
//! a named function, read with GNU binutils (`nm` and `objdump`), x86_64
//! only.
//!
//! A timing test ([`crate::ct`]) sees a branch on a secret, whose
//! mispredictions cost far more than its floor, but not a load from a small
//! table indexed by a secret, which stays in the first-level cache and
//! takes the same time for every index on one process's wall clock. This
//! check sees both in the code it is pointed at, whatever the timing on the
//! machine that runs it, and it sees the code at whatever optimisation
//! level the test was built with, so a build at a second level checks
//! that level too.
//!
//! It is syntactic, so what it can assert depends on the shape of the
//! function ([`Shape`]). Every shape forbids a division instruction
//! (`div`, `idiv`, and the floating-point and vector divisions), whose
//! latency depends on its operands on many x86-64 cores: the compiler
//! turns a division by a constant into a multiplication at most
//! optimisation levels, but not at every one (rustc keeps a `div` for a
//! `u32` divided by 58 at opt-level "z"). The two shapes that hold a
//! secret's every step also forbid a call and an indirect jump (code the
//! check does not see, or a jump table), and taking a table's address
//! (`lea disp(%rip), reg`), the first step of a load indexed by the data
//! from a table in the binary.
//!
//! - [`Shape::StraightLine`]: a function with no loop, such as a probe
//!   that runs one lookup on one value. It must have no conditional jump
//!   at all, and no memory operand but a fixed address (`disp(%rip)`, a
//!   constant) or a stack slot with no index register (based on `%rsp`,
//!   `%rbp`, or a register last set by `lea disp(%rsp), reg`): so no
//!   branch and no load or store whose address depends on the input.
//! - [`Shape::Loop`]: a function whose loops run over a public length.
//!   Loop control and a branch on the data cannot be told apart by syntax,
//!   so the rule is on layout: a conditional jump must go backward (a
//!   loop's back edge) or forward past the function's last backward jump
//!   (out of every loop). A forward conditional jump inside a loop, the
//!   shape the compiler gives a select turned into a branch (a jump over
//!   the instruction that sets the other value), fails. So does loop
//!   structure the rule does not know, such as an unrolled loop's
//!   remainder: a failure is code for a reader to inspect, not proof of a
//!   leak, and a pass does not rule out a branch on the data that is laid
//!   out as a back edge or an exit. Memory operands are not restricted,
//!   since the loop reads and writes its buffers by index.
//! - [`Shape::Branching`]: a function that branches, calls and indexes on
//!   values that are allowed to show in its time (a length, a value's
//!   magnitude), such as big-number arithmetic that grows as its value
//!   needs: only the division rule applies. The calls it makes are not
//!   followed, so a division in a callee (a runtime routine a target
//!   without a divide instruction would call, say) is not seen.
//!
//! The function must exist under its demangled path, so it must be kept out
//! of line (`#[inline(never)]`) and reachable from a test. A pass says
//! nothing about other functions, other builds or other architectures.

use std::path::Path;
use std::process::Command;

/// What [`violations`] asserts of a function; see the module
/// documentation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// No loop: no conditional jump, and no memory operand but a fixed
    /// address or an unindexed stack slot.
    StraightLine,
    /// Loops over a public length: every conditional jump a back edge or
    /// an exit from every loop.
    Loop,
    /// Branches, calls and indexes freely: no division instruction, and
    /// nothing else asserted.
    Branching,
}

/// One instruction: its address, mnemonic and operands, as objdump prints
/// them in AT&T syntax.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Insn {
    /// The address, in hexadecimal.
    pub addr: String,
    /// The first word objdump prints: the mnemonic, or a prefix (`lock`,
    /// `rep`, `cs`, `rex.W`) when there is one, the mnemonic then leading
    /// [`Insn::operands`].
    pub mnemonic: String,
    /// The operands, as printed.
    pub operands: String,
}

impl Insn {
    fn line(&self) -> String {
        format!("{}: {} {}", self.addr, self.mnemonic, self.operands)
    }
}

/// Parses objdump's `-d --no-show-raw-insn` output into instructions,
/// skipping headers, labels and padding (`nop`, `int3`).
pub fn parse_objdump(text: &str) -> Vec<Insn> {
    let mut out = Vec::new();
    for line in text.lines() {
        let Some((addr, rest)) = line.split_once(':') else {
            continue;
        };
        let addr = addr.trim();
        if addr.is_empty() || !addr.bytes().all(|b| b.is_ascii_hexdigit()) {
            continue;
        }
        // Drop a trailing comment (`# 1234 <symbol>`).
        let rest = rest.split('#').next().unwrap_or("").trim();
        if rest.is_empty() || rest.contains("nop") || rest.starts_with("int3") {
            continue;
        }
        let (mnemonic, operands) = match rest.split_once(char::is_whitespace) {
            Some((m, o)) => (m.to_string(), o.trim().to_string()),
            None => (rest.to_string(), String::new()),
        };
        out.push(Insn {
            addr: addr.to_string(),
            mnemonic,
            operands,
        });
    }
    out
}

/// Whether `mnemonic` divides: the integer `div` and `idiv` in every width,
/// and the floating-point and vector divisions (`divss`, `divpd`,
/// `vdivps`, `fdiv`, `fidiv` and their forms).
fn is_division(mnemonic: &str) -> bool {
    ["div", "idiv", "vdiv", "fdiv", "fidiv"]
        .iter()
        .any(|prefix| mnemonic.starts_with(prefix))
}

/// The instruction's own mnemonic: [`Insn::mnemonic`], or, when objdump
/// printed a prefix there (`rex.W`, `lock`, `data16`), the first word of
/// the operands that is not one.
fn operation(insn: &Insn) -> &str {
    const PREFIXES: [&str; 14] = [
        "rex", "lock", "rep", "repz", "repe", "repnz", "repne", "cs", "ds", "es", "ss", "data16",
        "addr32", "notrack",
    ];
    let is_prefix = |w: &str| PREFIXES.contains(&w) || w.starts_with("rex.");
    std::iter::once(insn.mnemonic.as_str())
        .chain(insn.operands.split_whitespace())
        .find(|w| !is_prefix(w))
        .unwrap_or("")
}

/// Whether `mnemonic` is a conditional transfer of control.
fn is_conditional_jump(mnemonic: &str) -> bool {
    let m = mnemonic;
    (m.starts_with('j') && !m.starts_with("jmp")) || m.starts_with("loop")
}

/// The memory operands of `operands` (AT&T: `disp(base,index,scale)`),
/// each from its `(` to its `)`.
fn memory_operands(operands: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = operands;
    while let Some((_, after_open)) = rest.split_once('(') {
        let Some((inside, after_close)) = after_open.split_once(')') else {
            break;
        };
        // `inside` and its parentheses are one subslice of `rest`.
        let start = rest
            .len()
            .saturating_sub(after_open.len())
            .saturating_sub(1);
        let end = rest.len().saturating_sub(after_close.len());
        out.push(rest.get(start..end).unwrap_or(inside));
        rest = after_close;
    }
    out
}

/// The 64-bit register that `name` (`%al`, `%ecx`, `%r9d`, `%rsp`) is
/// part of, or `None` for anything else (vector registers, `%rip`).
fn register_family(name: &str) -> Option<&'static str> {
    const LEGACY: [(&str, [&str; 5]); 8] = [
        ("%rax", ["%rax", "%eax", "%ax", "%al", "%ah"]),
        ("%rbx", ["%rbx", "%ebx", "%bx", "%bl", "%bh"]),
        ("%rcx", ["%rcx", "%ecx", "%cx", "%cl", "%ch"]),
        ("%rdx", ["%rdx", "%edx", "%dx", "%dl", "%dh"]),
        ("%rsi", ["%rsi", "%esi", "%si", "%sil", "%rsi"]),
        ("%rdi", ["%rdi", "%edi", "%di", "%dil", "%rdi"]),
        ("%rbp", ["%rbp", "%ebp", "%bp", "%bpl", "%rbp"]),
        ("%rsp", ["%rsp", "%esp", "%sp", "%spl", "%rsp"]),
    ];
    const NUMBERED: [&str; 8] = ["%r8", "%r9", "%r10", "%r11", "%r12", "%r13", "%r14", "%r15"];
    for (family, names) in LEGACY {
        if names.contains(&name) {
            return Some(family);
        }
    }
    NUMBERED.into_iter().find(|&family| {
        name.strip_prefix(family)
            .is_some_and(|rest| matches!(rest, "" | "d" | "w" | "b"))
    })
}

/// The register an instruction writes, if its last operand is a register
/// and its mnemonic is not one that only reads (`cmp`, `test`, `bt`).
fn written_register(insn: &Insn) -> Option<&'static str> {
    let m = insn.mnemonic.as_str();
    let reads_only = (m.starts_with("cmp") && !m.starts_with("cmpxchg"))
        || m.starts_with("test")
        || matches!(m, "bt" | "btw" | "btl" | "btq");
    if reads_only {
        return None;
    }
    let last = insn.operands.rsplit(',').next()?.trim();
    if last.ends_with(')') {
        return None;
    }
    register_family(last)
}

/// The target address of a jump (`b5746 <symbol+0x196>`), if it is direct.
fn jump_target(insn: &Insn) -> Option<u64> {
    let first = insn.operands.split_whitespace().next()?;
    u64::from_str_radix(first, 16).ok()
}

/// The violations of `shape`'s rules in `insns`, one line each; empty when
/// the code passes.
pub fn violations(insns: &[Insn], shape: Shape) -> Vec<String> {
    let mut out = Vec::new();
    let address = |insn: &Insn| u64::from_str_radix(&insn.addr, 16).ok();
    // The last backward jump: past it, a forward jump leaves every loop.
    let last_back_edge = insns
        .iter()
        .filter(|insn| is_conditional_jump(&insn.mnemonic) || insn.mnemonic.starts_with("jmp"))
        .filter_map(|insn| Some((address(insn)?, jump_target(insn)?)))
        .filter(|&(from, to)| to <= from)
        .map(|(from, _)| from)
        .max();
    // Registers that hold a stack address (StraightLine).
    let mut stack_registers: Vec<&str> = vec!["%rsp", "%rbp"];
    for insn in insns {
        let m = insn.mnemonic.as_str();
        if is_division(operation(insn)) {
            out.push(format!("a division: {}", insn.line()));
        }
        if shape != Shape::Branching {
            if m.starts_with("call") {
                out.push(format!("a call: {}", insn.line()));
            }
            if m.starts_with("jmp") && insn.operands.starts_with('*') {
                out.push(format!("an indirect jump: {}", insn.line()));
            }
            if m.starts_with("lea") && insn.operands.contains("(%rip)") {
                out.push(format!("a table's address taken: {}", insn.line()));
            }
        }
        match shape {
            Shape::Branching => {}
            Shape::StraightLine => {
                if is_conditional_jump(m) {
                    out.push(format!("a conditional jump: {}", insn.line()));
                }
                let mems = memory_operands(&insn.operands);
                if !m.starts_with("lea") {
                    for mem in &mems {
                        let base = mem.trim_start_matches('(').trim_end_matches(')');
                        let fixed = base == "%rip";
                        let stack = stack_registers.contains(&base);
                        if !fixed && !stack {
                            out.push(format!(
                                "a memory operand neither fixed nor an unindexed stack slot: {}",
                                insn.line()
                            ));
                        }
                    }
                }
                if let Some(written) = written_register(insn) {
                    let stack_address = m.starts_with("lea")
                        && mems.len() == 1
                        && stack_registers
                            .contains(&mems[0].trim_start_matches('(').trim_end_matches(')'));
                    stack_registers.retain(|&r| r != written);
                    if stack_address {
                        stack_registers.push(written);
                    }
                }
            }
            Shape::Loop => {
                if is_conditional_jump(m) {
                    let (Some(from), Some(to)) = (address(insn), jump_target(insn)) else {
                        out.push(format!(
                            "a conditional jump without a target: {}",
                            insn.line()
                        ));
                        continue;
                    };
                    let back_edge = to <= from;
                    let leaves_loops = last_back_edge.is_none_or(|last| to > last);
                    if !back_edge && !leaves_loops {
                        out.push(format!(
                            "a forward conditional jump that stays inside a loop: {}",
                            insn.line()
                        ));
                    }
                }
            }
        }
    }
    out
}

/// The address and size of the function whose demangled path is exactly
/// `symbol` in `exe`, by `nm`.
fn locate(exe: &Path, symbol: &str) -> Result<(u64, u64), String> {
    let output = Command::new("nm")
        .args(["--defined-only", "--demangle", "--print-size"])
        .arg(exe)
        .output()
        .map_err(|e| format!("cannot run nm (GNU binutils): {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "nm failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut found = None;
    for line in text.lines() {
        let mut fields = line.splitn(4, ' ');
        let (Some(addr), Some(size), Some(kind), Some(name)) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        if name != symbol || !matches!(kind, "t" | "T") {
            continue;
        }
        let parse = |s: &str| u64::from_str_radix(s, 16).map_err(|e| format!("{line}: {e}"));
        if found.replace((parse(addr)?, parse(size)?)).is_some() {
            return Err(format!(
                "{symbol} is defined more than once in {}",
                exe.display()
            ));
        }
    }
    found.ok_or_else(|| {
        format!(
            "{symbol} is not in {} (inlined, renamed or unused?)",
            exe.display()
        )
    })
}

/// The instructions of the function whose demangled path is exactly
/// `symbol` in the running executable.
pub fn disassemble(symbol: &str) -> Result<Vec<Insn>, String> {
    let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    let (addr, size) = locate(&exe, symbol)?;
    let stop = addr
        .checked_add(size)
        .ok_or_else(|| format!("{symbol}: size overflows"))?;
    let output = Command::new("objdump")
        .args([
            "-d",
            "--no-show-raw-insn",
            &format!("--start-address={addr:#x}"),
            &format!("--stop-address={stop:#x}"),
        ])
        .arg(&exe)
        .output()
        .map_err(|e| format!("cannot run objdump (GNU binutils): {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "objdump failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let insns = parse_objdump(&String::from_utf8_lossy(&output.stdout));
    if insns.is_empty() {
        return Err(format!("{symbol}: objdump printed no instructions"));
    }
    Ok(insns)
}

/// Disassembles each `(symbol, shape)` and panics with every violation
/// found and the code it was found in; prints one line a function, marked
/// `ct_disasm |`, when all pass.
pub fn assert_clean(functions: &[(&str, Shape)]) {
    let mut failures = String::new();
    for &(symbol, shape) in functions {
        let insns = disassemble(symbol).unwrap_or_else(|e| panic!("{e}"));
        let found = violations(&insns, shape);
        if found.is_empty() {
            println!(
                "ct_disasm | {symbol} | {shape:?} | {} instructions | pass",
                insns.len()
            );
            continue;
        }
        failures.push_str(&format!("\n{symbol} ({shape:?}):\n"));
        for v in &found {
            failures.push_str(&format!("  {v}\n"));
        }
        failures.push_str("  code:\n");
        for insn in &insns {
            failures.push_str(&format!("    {}\n", insn.line()));
        }
    }
    assert!(failures.is_empty(), "ct_disasm found:{failures}");
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "
/x/target/release/deps/purse_hdkeychain-0123:     file format elf64-x86-64


Disassembly of section .text:

00000000000aa3a0 <_RNvNtCs1_16purse_hdkeychain6base5810map_digits>:
   aa3a0:\tmovzbl %dil,%eax
   aa3a4:\tlea    -0x31(%rax),%ecx
   aa3a7:\tmov    %rcx,-0x8(%rsp)
   aa3ac:\tmov    -0x8(%rsp),%rcx
   aa3b2:\tcmp    %r11b,(%rsi,%rbx,1)
   aa3bc:\tje     aa3c0 <_RNvNtCs1_16purse_hdkeychain6base5810map_digits+0x20>
   aa3be:\tmovdqa 0x1234(%rip),%xmm0        # bb000 <anon>
   aa3c0:\tlea    0x5678(%rip),%r8        # cc000 <anon>
   aa3c7:\tcall   aa000 <helper>
   aa3ca:\tjmp    *%rax
   aa3cc:\tcs nopw 0x0(%rax,%rax,1)
   aa3d6:\tret
";

    fn kinds(v: &[String]) -> Vec<&str> {
        v.iter()
            .map(|s| s.split(':').next().unwrap_or(""))
            .collect()
    }

    #[test]
    fn parses_objdump_output() {
        let insns = parse_objdump(SAMPLE);
        assert_eq!(insns.len(), 11, "{insns:?}");
        assert_eq!(insns[0].addr, "aa3a0");
        assert_eq!(insns[0].mnemonic, "movzbl");
        assert_eq!(insns[0].operands, "%dil,%eax");
        assert_eq!(insns[6].operands, "0x1234(%rip),%xmm0");
        assert_eq!(jump_target(&insns[5]), Some(0xaa3c0));
        assert_eq!(insns[10].mnemonic, "ret");
    }

    #[test]
    fn straight_line_rules() {
        let v = violations(&parse_objdump(SAMPLE), Shape::StraightLine);
        assert_eq!(
            kinds(&v),
            [
                "a memory operand neither fixed nor an unindexed stack slot",
                "a conditional jump",
                "a table's address taken",
                "a call",
                "an indirect jump",
            ],
            "{v:#?}"
        );
    }

    #[test]
    fn stack_slots_through_a_register() {
        let code = "
   10:\tlea    -0x8(%rsp),%rax
   14:\tmov    %rdx,(%rax)
   17:\tmov    (%rax),%rax
   1a:\tmov    %rdx,(%rax)
   1d:\tlea    -0x10(%rsp),%rcx
   21:\tmov    (%rcx,%rdx,1),%al
   24:\tret
";
        let v = violations(&parse_objdump(code), Shape::StraightLine);
        // The slot through %rax is allowed until %rax is overwritten by a
        // load; an indexed stack slot is a table on the stack.
        assert_eq!(v.len(), 2, "{v:#?}");
        assert!(
            v[0].starts_with("a memory operand") && v[0].contains("1a:"),
            "{v:#?}"
        );
        assert!(
            v[1].starts_with("a memory operand") && v[1].contains("21:"),
            "{v:#?}"
        );
    }

    #[test]
    fn loop_rules() {
        let v = violations(&parse_objdump(SAMPLE), Shape::Loop);
        assert_eq!(
            kinds(&v),
            ["a table's address taken", "a call", "an indirect jump"],
            "{v:#?}"
        );
        // A guard that leaves the loop and a back edge pass; a jump over
        // one instruction inside the loop (a select made a branch) fails.
        let code = "
   10:\ttest   %rsi,%rsi
   13:\tje     40 <f+0x30>
   15:\tmovzbl (%rdi,%rcx,1),%eax
   19:\tcmp    $0x39,%al
   1b:\tja     1f <f+0xf>
   1d:\txor    %ebp,%ebp
   1f:\tmov    %bpl,(%rdx,%rcx,1)
   23:\tinc    %rcx
   26:\tcmp    %rsi,%rcx
   29:\tjne    15 <f+0x5>
   40:\tret
";
        let v = violations(&parse_objdump(code), Shape::Loop);
        assert_eq!(
            kinds(&v),
            ["a forward conditional jump that stays inside a loop"]
        );
        assert!(v[0].contains("1b:"), "{v:#?}");
    }

    #[test]
    fn clean_code_passes() {
        let clean = "
   10:\tmovzbl %dil,%eax
   14:\tsub    $0x31,%eax
   17:\tshr    $0x1f,%eax
   1a:\tmov    %rax,-0x8(%rsp)
   1f:\tmov    -0x8(%rsp),%rcx
   24:\tand    0x10(%rip),%ecx
   2a:\tcmovne %ecx,%eax
   2d:\tret
";
        let insns = parse_objdump(clean);
        assert_eq!(insns.len(), 8);
        assert!(violations(&insns, Shape::StraightLine).is_empty());
        assert!(violations(&insns, Shape::Loop).is_empty());
    }

    #[test]
    fn divisions_fail_every_shape() {
        // The inner loop rustc emits at opt-level "z" for `val % 58` and
        // `val / 58` on a u32, with a call and a table's address beside it,
        // which only the stricter shapes reject.
        let code = "
   10:\tmovzbl (%rcx,%rdi,1),%eax
   14:\tshl    $0x8,%eax
   17:\tadd    %ebp,%eax
   19:\txor    %edx,%edx
   1b:\tdiv    %ebx
   1d:\tmov    %dl,(%rcx,%rdi,1)
   20:\tlea    0x5678(%rip),%rdx
   27:\tcall   aa000 <core::panicking::panic_bounds_check>
   2c:\tret
";
        let insns = parse_objdump(code);
        let v = violations(&insns, Shape::Branching);
        assert_eq!(kinds(&v), ["a division"], "{v:#?}");
        assert!(v[0].contains("1b:"), "{v:#?}");
        for shape in [Shape::StraightLine, Shape::Loop] {
            assert!(kinds(&violations(&insns, shape)).contains(&"a division"));
        }
        // The multiplication and shift that replace it pass.
        let clean = "
   10:\timul   $0x469f,%eax,%edx
   16:\tshr    $0x14,%edx
   19:\timul   $0x3a,%edx,%esi
   1c:\tsub    %esi,%eax
   1e:\tcall   aa000 <core::panicking::panic_bounds_check>
   23:\tret
";
        assert!(violations(&parse_objdump(clean), Shape::Branching).is_empty());
        for m in [
            "div", "divl", "divq", "idivl", "idivq", "divss", "divsd", "divps", "vdivpd", "fdivp",
            "fidivl",
        ] {
            assert!(is_division(m), "{m}");
        }
        // A prefix printed where the mnemonic goes does not hide one.
        let prefixed = parse_objdump("   10:\trex.W div %rbx\n   13:\tdata16 lock idivw %cx\n");
        assert_eq!(prefixed[0].mnemonic, "rex.W");
        assert_eq!(operation(&prefixed[0]), "div");
        assert_eq!(operation(&prefixed[1]), "idivw");
        assert_eq!(violations(&prefixed, Shape::Branching).len(), 2);
        for m in ["imul", "mul", "mulx", "shr", "sar", "dpps"] {
            assert!(!is_division(m), "{m}");
        }
    }

    #[test]
    fn finds_conditional_jumps_of_every_kind() {
        for m in ["je", "jne", "ja", "jbe", "js", "jrcxz", "loop", "loopne"] {
            assert!(is_conditional_jump(m), "{m}");
        }
        for m in ["jmp", "jmpq", "cmovne", "sete", "call"] {
            assert!(!is_conditional_jump(m), "{m}");
        }
    }

    #[test]
    fn register_families() {
        assert_eq!(register_family("%al"), Some("%rax"));
        assert_eq!(register_family("%ecx"), Some("%rcx"));
        assert_eq!(register_family("%sil"), Some("%rsi"));
        assert_eq!(register_family("%r9d"), Some("%r9"));
        assert_eq!(register_family("%r10"), Some("%r10"));
        assert_eq!(register_family("%r1"), None);
        assert_eq!(register_family("%xmm0"), None);
    }
}
