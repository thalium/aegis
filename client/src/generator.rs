use iced_x86::{FlowControl, Instruction, Mnemonic, Register};
use libaegis::cpu::{CpuState, FlagState};
use rand::{
    rngs::ThreadRng,
    seq::{IndexedRandom, IteratorRandom},
    Rng,
};

enum Operand {
    Register(Register),
    Immediate(u32),
    None,
}

fn random_reg_8(rng: &mut ThreadRng) -> Operand {
    use Register::*;
    const REGISTERS: &[Register] = &[
        AL, CL, DL, BL, AH, CH, DH, BH, BPL, SIL, DIL, R8L, R9L, R10L, R11L, R12L, R13L, R14L,
    ];
    Operand::Register(REGISTERS.choose(rng).unwrap().clone())
}

fn random_reg_16(rng: &mut ThreadRng) -> Operand {
    use Register::*;
    const REGISTERS: &[Register] = &[
        AX, CX, DX, BX, BP, SI, DI, R8W, R9W, R10W, R11W, R12W, R13W, R14W,
    ];
    Operand::Register(REGISTERS.choose(rng).unwrap().clone())
}

fn random_reg_32(rng: &mut ThreadRng) -> Operand {
    use Register::*;
    const REGISTERS: &[Register] = &[
        EAX, ECX, EDX, EBX, EBP, ESI, EDI, R8D, R9D, R10D, R11D, R12D, R13D, R14D,
    ];
    Operand::Register(REGISTERS.choose(rng).unwrap().clone())
}

fn random_reg_64(rng: &mut ThreadRng) -> Operand {
    use Register::*;
    const REGISTERS: &[Register] = &[
        RAX, RCX, RDX, RBX, RBP, RSI, RDI, R8, R9, R10, R11, R12, R13, R14,
    ];
    Operand::Register(REGISTERS.choose(rng).unwrap().clone())
}

/// Creates a random immediate with "interesting values"
fn random_imm(rng: &mut ThreadRng) -> Operand {
    const VALUES: &[u32] = &[
        0,
        1,
        2,
        3,
        4,
        5,
        6,
        7,
        8,
        0x100,
        u32::MAX,
        u32::MAX - 1,
        u32::MAX - 2,
        u32::MAX - 3,
        u32::MAX - 4,
        u32::MAX - 5,
        u32::MAX - 6,
        u32::MAX - 7,
        0x7F,
        0x7FFF,
        0x7FFFFFFF,
        0xF,
        0xFF,
    ];

    Operand::Immediate(VALUES.choose(rng).unwrap().clone())
}

pub fn is_blacklisted(mnemonic: Mnemonic) -> bool {
    use iced_x86::Mnemonic::*;
    match mnemonic {
        Std | Verr | Verw | Lsl | Lar => true,

        _ => false,
    }
}

pub fn random_insn(rng: &mut ThreadRng) -> Instruction {
    loop {
        let code = iced_x86::Code::values().choose(rng).unwrap();
        let opcode = code.op_code();

        // Filter
        if code.flow_control() != FlowControl::Next
            || code.is_stack_instruction()
            // || code.cpuid_features().len() != 1
            || !(
                code.cpuid_features().contains(&iced_x86::CpuidFeature::INTEL8086)
                || code.cpuid_features().contains(&iced_x86::CpuidFeature::INTEL186)
                || code.cpuid_features().contains(&iced_x86::CpuidFeature::INTEL286)
                || code.cpuid_features().contains(&iced_x86::CpuidFeature::INTEL386)
            )
            || opcode.is_privileged()
            || !opcode.is_instruction()
            || !opcode.is_available_in_mode(64)
            || opcode.op_count() > 2
            || is_blacklisted(code.mnemonic())
        {
            continue;
        }

        let ops = opcode
            .op_kinds()
            .iter()
            .map(|kind| {
                match kind {
                    iced_x86::OpCodeOperandKind::None => Operand::None,

                    iced_x86::OpCodeOperandKind::r8_reg
                    | iced_x86::OpCodeOperandKind::r8_or_mem => random_reg_8(rng),

                    iced_x86::OpCodeOperandKind::r16_reg
                    | iced_x86::OpCodeOperandKind::r16_or_mem => random_reg_16(rng),

                    iced_x86::OpCodeOperandKind::r32_reg
                    | iced_x86::OpCodeOperandKind::r32_or_mem => random_reg_32(rng),

                    iced_x86::OpCodeOperandKind::r64_reg
                    | iced_x86::OpCodeOperandKind::r64_or_mem => random_reg_64(rng),

                    // Fixed
                    iced_x86::OpCodeOperandKind::al => Operand::Register(Register::AL),
                    iced_x86::OpCodeOperandKind::cl => Operand::Register(Register::CL),
                    iced_x86::OpCodeOperandKind::ax => Operand::Register(Register::AX),
                    iced_x86::OpCodeOperandKind::dx => Operand::Register(Register::DX),
                    iced_x86::OpCodeOperandKind::eax => Operand::Register(Register::EAX),
                    iced_x86::OpCodeOperandKind::rax => Operand::Register(Register::RAX),

                    // Immediates
                    // TODO: add more immediate types
                    iced_x86::OpCodeOperandKind::imm8
                    | iced_x86::OpCodeOperandKind::imm8_const_1
                    | iced_x86::OpCodeOperandKind::imm8sex16
                    | iced_x86::OpCodeOperandKind::imm8sex32
                    | iced_x86::OpCodeOperandKind::imm8sex64
                    | iced_x86::OpCodeOperandKind::imm16
                    | iced_x86::OpCodeOperandKind::imm32
                    | iced_x86::OpCodeOperandKind::imm32sex64
                    | iced_x86::OpCodeOperandKind::imm64 => random_imm(rng),

                    _ => Operand::None,
                }
            })
            .collect::<Vec<Operand>>();

        let insn = match ops[0..opcode.op_count() as usize] {
            [] => Ok(Instruction::with(code)),

            [Operand::Register(r)] => Instruction::with1(code, r.clone()).map_err(|_| ()),

            [Operand::Immediate(i)] => Instruction::with1(code, i.clone()).map_err(|_| ()),

            [Operand::Register(r1), Operand::Register(r2)] => {
                Instruction::with2(code, r1.clone(), r2.clone()).map_err(|_| ())
            }

            [Operand::Register(r), Operand::Immediate(i)] => {
                Instruction::with2(code, r.clone(), i.clone()).map_err(|_| ())
            }

            [Operand::Immediate(i), Operand::Register(r)] => {
                Instruction::with2(code, i.clone(), r.clone()).map_err(|_| ())
            }

            [Operand::Immediate(i1), Operand::Immediate(i2)] => {
                Instruction::with2(code, i1.clone(), i2.clone()).map_err(|_| ())
            }

            _ => Err(()),
        };

        if let Ok(insn) = insn {
            return insn;
        }
    }
}

pub fn random_state(rng: &mut ThreadRng, mut zero_state: CpuState) -> CpuState {
    zero_state.flags = FlagState(0);

    for bit in [0, 2, 4, 6, 7, 11] {
        if rng.random_bool(0.5) {
            zero_state.flags.0 |= 1 << bit;
        }
    }

    zero_state.gpr.rax = rng.random();
    zero_state.gpr.rbx = rng.random();
    zero_state.gpr.rcx = rng.random();
    zero_state.gpr.rdx = rng.random();
    zero_state.gpr.rsi = rng.random();
    zero_state.gpr.rdi = rng.random();

    zero_state.gpr.rbp = rng.random();

    zero_state.gpr.r8 = rng.random();
    zero_state.gpr.r9 = rng.random();
    zero_state.gpr.r10 = rng.random();
    zero_state.gpr.r11 = rng.random();
    zero_state.gpr.r12 = rng.random();
    zero_state.gpr.r13 = rng.random();
    zero_state.gpr.r14 = rng.random();

    zero_state
}
