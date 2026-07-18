#![deny(unsafe_code)]
#![deny(clippy::all)]
#![deny(unreachable_pub)]

//! A bounded, slice-driven RV64 machine for the AOS Realm Linux backend.
//!
//! This crate is intentionally below the Linux compatibility policy. It owns
//! guest CPU state, admitted RAM, and virtual hardware. The outer Realm owns
//! scheduling, authority, image admission, persistence, and all host effects.

use std::{collections::VecDeque, fmt};

/// Machine profile whose future device tree and Linux image are versioned together.
pub const MACHINE_MODEL: &str = "aos-rv64-virt-v0";

/// Guest physical address at which admitted RAM begins.
pub const DRAM_BASE: u64 = 0x8000_0000;

/// Guest physical base of the 16550-compatible serial device.
pub const UART_BASE: u64 = 0x1000_0000;

/// Guest physical base of the SiFive/QEMU-compatible test finisher.
pub const TEST_FINISHER_BASE: u64 = 0x0010_0000;

const UART_SIZE: u64 = 0x100;
const TEST_FINISHER_SIZE: u64 = 0x1000;
const UART_RECEIVE: u64 = 0;
const UART_TRANSMIT: u64 = 0;
const UART_INTERRUPT_IDENTIFICATION: u64 = 2;
const UART_LINE_STATUS: u64 = 5;
const UART_LINE_STATUS_DATA_READY: u8 = 1;
const UART_LINE_STATUS_TRANSMIT_EMPTY: u8 = (1 << 5) | (1 << 6);
const MIN_RAM_BYTES: usize = 4096;
const MAX_RAM_BYTES: usize = 256 * 1024 * 1024;
const MAX_CONSOLE_BYTES: usize = 16 * 1024 * 1024;

/// Explicit resource admission for one virtual machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MachineConfig {
    /// Contiguous guest RAM. It must be page-aligned and remains within the outer
    /// capsule's own Wasm memory limit.
    pub ram_bytes: usize,
    /// Maximum serial output retained for the current machine execution.
    pub max_console_bytes: usize,
}

impl Default for MachineConfig {
    fn default() -> Self {
        Self {
            ram_bytes: 64 * 1024,
            max_console_bytes: 64 * 1024,
        }
    }
}

/// Machine construction or image-admission failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MachineError {
    /// RAM is too small, too large, or not aligned to a 4 KiB guest page.
    InvalidRamBytes(usize),
    /// The retained serial-output limit exceeds the hard machine cap.
    InvalidConsoleBytes(usize),
    /// A program image is empty or does not fit in admitted guest RAM.
    InvalidProgramBytes { image: usize, ram: usize },
}

impl fmt::Display for MachineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRamBytes(bytes) => write!(
                f,
                "guest RAM must be 4 KiB aligned and between {MIN_RAM_BYTES} and {MAX_RAM_BYTES} bytes, got {bytes}"
            ),
            Self::InvalidConsoleBytes(bytes) => write!(
                f,
                "console limit must not exceed {MAX_CONSOLE_BYTES} bytes, got {bytes}"
            ),
            Self::InvalidProgramBytes { image, ram } => {
                write!(
                    f,
                    "guest image is {image} bytes but admitted RAM is {ram} bytes"
                )
            }
        }
    }
}

impl std::error::Error for MachineError {}

/// RISC-V privilege level retained as part of guest architectural state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Privilege {
    /// Unprivileged application execution.
    User,
    /// Linux kernel execution.
    Supervisor,
    /// Firmware and reset execution.
    Machine,
}

/// Stable architectural trap reported to the outer Realm scheduler.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MachineTrap {
    /// The next instruction address violates the RV64I four-byte alignment.
    InstructionAddressMisaligned { address: u64 },
    /// The next instruction is outside admitted RAM.
    InstructionAccessFault { address: u64 },
    /// The instruction is not implemented by this machine profile.
    IllegalInstruction { pc: u64, instruction: u32 },
    /// A load address is not naturally aligned for its width.
    LoadAddressMisaligned { address: u64, bytes: u8 },
    /// A load address is outside RAM and admitted MMIO.
    LoadAccessFault { address: u64, bytes: u8 },
    /// A store address is not naturally aligned for its width.
    StoreAddressMisaligned { address: u64, bytes: u8 },
    /// A store address is outside RAM and admitted MMIO.
    StoreAccessFault { address: u64, bytes: u8 },
    /// An environment call crossed into the outer machine boundary.
    EnvironmentCall { privilege: Privilege },
    /// Guest execution reached an `ebreak` instruction.
    Breakpoint { pc: u64 },
    /// Serial output exceeded its admitted retained-byte ceiling.
    ConsoleLimit { limit: usize },
}

impl fmt::Display for MachineTrap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InstructionAddressMisaligned { address } => {
                write!(f, "instruction address {address:#x} is misaligned")
            }
            Self::InstructionAccessFault { address } => {
                write!(f, "instruction access fault at {address:#x}")
            }
            Self::IllegalInstruction { pc, instruction } => {
                write!(f, "illegal instruction {instruction:#010x} at {pc:#x}")
            }
            Self::LoadAddressMisaligned { address, bytes } => {
                write!(f, "{bytes}-byte load address {address:#x} is misaligned")
            }
            Self::LoadAccessFault { address, bytes } => {
                write!(f, "{bytes}-byte load access fault at {address:#x}")
            }
            Self::StoreAddressMisaligned { address, bytes } => {
                write!(f, "{bytes}-byte store address {address:#x} is misaligned")
            }
            Self::StoreAccessFault { address, bytes } => {
                write!(f, "{bytes}-byte store access fault at {address:#x}")
            }
            Self::EnvironmentCall { privilege } => {
                write!(f, "environment call from {privilege:?} mode")
            }
            Self::Breakpoint { pc } => write!(f, "breakpoint at {pc:#x}"),
            Self::ConsoleLimit { limit } => {
                write!(f, "console output exceeded {limit} bytes")
            }
        }
    }
}

impl std::error::Error for MachineTrap {}

/// Terminal value written by firmware or a test guest to the standard finisher.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HaltStatus {
    /// Whether the standard pass value was written.
    pub passed: bool,
    /// Guest-provided failure code, or zero for success.
    pub code: u32,
}

/// Result of one bounded scheduling slice.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SliceOutcome {
    /// The instruction budget ended while the guest remained runnable.
    Yielded,
    /// The guest wrote a terminal value to the standard finisher.
    Halted(HaltStatus),
    /// The guest crossed an unsupported or invalid architectural boundary.
    Trapped(MachineTrap),
}

/// Exact accounting and serial bytes produced by one scheduling slice.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SliceReport {
    /// Result at the end of this slice.
    pub outcome: SliceOutcome,
    /// Instructions retired during this slice.
    pub instructions_retired: u64,
    /// Total instructions retired since the last image load.
    pub total_instructions_retired: u64,
    /// Serial output produced since the previous slice report.
    pub console: Vec<u8>,
}

#[derive(Clone, Debug)]
struct Cpu {
    registers: [u64; 32],
    pc: u64,
    privilege: Privilege,
}

impl Cpu {
    fn new() -> Self {
        Self {
            registers: [0; 32],
            pc: DRAM_BASE,
            privilege: Privilege::Machine,
        }
    }

    fn reset(&mut self) {
        self.registers.fill(0);
        self.pc = DRAM_BASE;
        self.privilege = Privilege::Machine;
    }

    fn read(&self, register: usize) -> u64 {
        self.registers[register]
    }

    fn write(&mut self, register: usize, value: u64) {
        if register != 0 {
            self.registers[register] = value;
        }
    }
}

#[derive(Clone, Debug)]
enum RunState {
    Runnable,
    Halted(HaltStatus),
    Trapped(MachineTrap),
}

#[derive(Debug)]
struct Devices {
    ram: Vec<u8>,
    console_input: VecDeque<u8>,
    console_output: Vec<u8>,
    console_reported: usize,
    max_console_bytes: usize,
}

impl Devices {
    fn new(config: MachineConfig) -> Self {
        Self {
            ram: vec![0; config.ram_bytes],
            console_input: VecDeque::new(),
            console_output: Vec::new(),
            console_reported: 0,
            max_console_bytes: config.max_console_bytes,
        }
    }

    fn reset(&mut self) {
        self.ram.fill(0);
        self.console_input.clear();
        self.console_output.clear();
        self.console_reported = 0;
    }

    fn load_program(&mut self, program: &[u8]) {
        self.ram[..program.len()].copy_from_slice(program);
    }

    fn take_new_console(&mut self) -> Vec<u8> {
        let bytes = self.console_output[self.console_reported..].to_vec();
        self.console_reported = self.console_output.len();
        bytes
    }

    fn read(&mut self, address: u64, bytes: u8) -> Result<u64, MachineTrap> {
        if let Some(offset) = address
            .checked_sub(UART_BASE)
            .filter(|offset| *offset < UART_SIZE)
        {
            if bytes != 1 {
                return Err(MachineTrap::LoadAccessFault { address, bytes });
            }
            return Ok(match offset {
                UART_RECEIVE => self.console_input.pop_front().unwrap_or_default() as u64,
                UART_INTERRUPT_IDENTIFICATION => 1,
                UART_LINE_STATUS => {
                    let ready = if self.console_input.is_empty() {
                        0
                    } else {
                        UART_LINE_STATUS_DATA_READY
                    };
                    (UART_LINE_STATUS_TRANSMIT_EMPTY | ready) as u64
                }
                _ => 0,
            });
        }

        let range = self
            .ram_range(address, bytes)
            .ok_or(MachineTrap::LoadAccessFault { address, bytes })?;
        let mut value = 0_u64;
        for (shift, byte) in self.ram[range].iter().enumerate() {
            value |= u64::from(*byte) << (shift * 8);
        }
        Ok(value)
    }

    fn write(
        &mut self,
        address: u64,
        value: u64,
        bytes: u8,
    ) -> Result<Option<HaltStatus>, MachineTrap> {
        if let Some(offset) = address
            .checked_sub(UART_BASE)
            .filter(|offset| *offset < UART_SIZE)
        {
            if bytes != 1 || offset != UART_TRANSMIT {
                return Err(MachineTrap::StoreAccessFault { address, bytes });
            }
            if self.console_output.len() == self.max_console_bytes {
                return Err(MachineTrap::ConsoleLimit {
                    limit: self.max_console_bytes,
                });
            }
            self.console_output.push(value as u8);
            return Ok(None);
        }

        if (TEST_FINISHER_BASE..TEST_FINISHER_BASE + TEST_FINISHER_SIZE).contains(&address) {
            if bytes != 4 || address != TEST_FINISHER_BASE {
                return Err(MachineTrap::StoreAccessFault { address, bytes });
            }
            let value = value as u32;
            if value == 0x5555 {
                return Ok(Some(HaltStatus {
                    passed: true,
                    code: 0,
                }));
            }
            if value & 0xffff == 0x3333 {
                return Ok(Some(HaltStatus {
                    passed: false,
                    code: value >> 16,
                }));
            }
            return Ok(None);
        }

        let range = self
            .ram_range(address, bytes)
            .ok_or(MachineTrap::StoreAccessFault { address, bytes })?;
        for (shift, byte) in self.ram[range].iter_mut().enumerate() {
            *byte = (value >> (shift * 8)) as u8;
        }
        Ok(None)
    }

    fn ram_range(&self, address: u64, bytes: u8) -> Option<std::ops::Range<usize>> {
        let offset = address.checked_sub(DRAM_BASE)?;
        let start = usize::try_from(offset).ok()?;
        let end = start.checked_add(usize::from(bytes))?;
        (end <= self.ram.len()).then_some(start..end)
    }
}

/// An admitted RV64 machine whose execution can only advance in explicit slices.
#[derive(Debug)]
pub struct Machine {
    config: MachineConfig,
    cpu: Cpu,
    devices: Devices,
    state: RunState,
    instructions_retired: u64,
}

impl Machine {
    /// Admit resources and construct a reset RV64 machine.
    pub fn new(config: MachineConfig) -> Result<Self, MachineError> {
        if !(MIN_RAM_BYTES..=MAX_RAM_BYTES).contains(&config.ram_bytes)
            || !config.ram_bytes.is_multiple_of(MIN_RAM_BYTES)
        {
            return Err(MachineError::InvalidRamBytes(config.ram_bytes));
        }
        if config.max_console_bytes > MAX_CONSOLE_BYTES {
            return Err(MachineError::InvalidConsoleBytes(config.max_console_bytes));
        }
        Ok(Self {
            config,
            cpu: Cpu::new(),
            devices: Devices::new(config),
            state: RunState::Runnable,
            instructions_retired: 0,
        })
    }

    /// Reset the machine and copy a raw RV64 image to [`DRAM_BASE`].
    pub fn load_program(&mut self, program: &[u8]) -> Result<(), MachineError> {
        if program.is_empty() || program.len() > self.config.ram_bytes {
            return Err(MachineError::InvalidProgramBytes {
                image: program.len(),
                ram: self.config.ram_bytes,
            });
        }
        self.cpu.reset();
        self.devices.reset();
        self.devices.load_program(program);
        self.state = RunState::Runnable;
        self.instructions_retired = 0;
        Ok(())
    }

    /// Add bytes that the guest may consume from the serial receive register.
    pub fn push_console_input(&mut self, bytes: &[u8]) {
        self.devices.console_input.extend(bytes.iter().copied());
    }

    /// Read one architectural integer register. Register zero is always zero.
    #[must_use]
    pub fn register(&self, register: usize) -> Option<u64> {
        self.cpu.registers.get(register).copied()
    }

    /// Current guest program counter.
    #[must_use]
    pub const fn pc(&self) -> u64 {
        self.cpu.pc
    }

    /// Current guest privilege level.
    #[must_use]
    pub const fn privilege(&self) -> Privilege {
        self.cpu.privilege
    }

    /// Run at most `instruction_budget` instructions and return control to the Realm.
    pub fn run_slice(&mut self, instruction_budget: u64) -> SliceReport {
        let mut retired = 0_u64;
        while retired < instruction_budget && matches!(self.state, RunState::Runnable) {
            match self.step() {
                Ok(halt) => {
                    retired = retired.saturating_add(1);
                    self.instructions_retired = self.instructions_retired.saturating_add(1);
                    if let Some(status) = halt {
                        self.state = RunState::Halted(status);
                    }
                }
                Err(trap) => self.state = RunState::Trapped(trap),
            }
        }

        let outcome = match &self.state {
            RunState::Runnable => SliceOutcome::Yielded,
            RunState::Halted(status) => SliceOutcome::Halted(*status),
            RunState::Trapped(trap) => SliceOutcome::Trapped(trap.clone()),
        };
        SliceReport {
            outcome,
            instructions_retired: retired,
            total_instructions_retired: self.instructions_retired,
            console: self.devices.take_new_console(),
        }
    }

    fn step(&mut self) -> Result<Option<HaltStatus>, MachineTrap> {
        let pc = self.cpu.pc;
        if pc & 3 != 0 {
            return Err(MachineTrap::InstructionAddressMisaligned { address: pc });
        }
        let instruction = self
            .devices
            .read(pc, 4)
            .map_err(|_| MachineTrap::InstructionAccessFault { address: pc })?
            as u32;
        let opcode = instruction & 0x7f;
        let rd = ((instruction >> 7) & 0x1f) as usize;
        let funct3 = (instruction >> 12) & 0x7;
        let rs1 = ((instruction >> 15) & 0x1f) as usize;
        let rs2 = ((instruction >> 20) & 0x1f) as usize;
        let funct7 = instruction >> 25;
        let mut next_pc = pc.wrapping_add(4);
        let mut halt = None;

        match opcode {
            0x03 => {
                let address = self.cpu.read(rs1).wrapping_add(immediate_i(instruction));
                let (bytes, signed) = match funct3 {
                    0 => (1, true),
                    1 => (2, true),
                    2 => (4, true),
                    3 => (8, true),
                    4 => (1, false),
                    5 => (2, false),
                    6 => (4, false),
                    _ => return Err(illegal(pc, instruction)),
                };
                ensure_aligned(address, bytes, false)?;
                let value = self.devices.read(address, bytes)?;
                let value = if signed {
                    sign_extend(value, u32::from(bytes) * 8)
                } else {
                    value
                };
                self.cpu.write(rd, value);
            }
            0x0f => {
                if funct3 > 1 {
                    return Err(illegal(pc, instruction));
                }
            }
            0x13 => self.execute_op_imm(instruction, rd, rs1, funct3, pc)?,
            0x17 => self
                .cpu
                .write(rd, pc.wrapping_add(immediate_u(instruction))),
            0x1b => self.execute_op_imm_32(instruction, rd, rs1, funct3, pc)?,
            0x23 => {
                let bytes = match funct3 {
                    0 => 1,
                    1 => 2,
                    2 => 4,
                    3 => 8,
                    _ => return Err(illegal(pc, instruction)),
                };
                let address = self.cpu.read(rs1).wrapping_add(immediate_s(instruction));
                ensure_aligned(address, bytes, true)?;
                halt = self.devices.write(address, self.cpu.read(rs2), bytes)?;
            }
            0x33 => self.execute_op(instruction, rd, rs1, rs2, funct3, funct7, pc)?,
            0x37 => self.cpu.write(rd, immediate_u(instruction)),
            0x3b => self.execute_op_32(instruction, rd, rs1, rs2, funct3, funct7, pc)?,
            0x63 => {
                let lhs = self.cpu.read(rs1);
                let rhs = self.cpu.read(rs2);
                let take = match funct3 {
                    0 => lhs == rhs,
                    1 => lhs != rhs,
                    4 => (lhs as i64) < (rhs as i64),
                    5 => (lhs as i64) >= (rhs as i64),
                    6 => lhs < rhs,
                    7 => lhs >= rhs,
                    _ => return Err(illegal(pc, instruction)),
                };
                if take {
                    let target = pc.wrapping_add(immediate_b(instruction));
                    ensure_instruction_aligned(target)?;
                    next_pc = target;
                }
            }
            0x67 => {
                if funct3 != 0 {
                    return Err(illegal(pc, instruction));
                }
                let target = self.cpu.read(rs1).wrapping_add(immediate_i(instruction)) & !1;
                ensure_instruction_aligned(target)?;
                self.cpu.write(rd, next_pc);
                next_pc = target;
            }
            0x6f => {
                let target = pc.wrapping_add(immediate_j(instruction));
                ensure_instruction_aligned(target)?;
                self.cpu.write(rd, next_pc);
                next_pc = target;
            }
            0x73 => match instruction {
                0x0000_0073 => {
                    return Err(MachineTrap::EnvironmentCall {
                        privilege: self.cpu.privilege,
                    });
                }
                0x0010_0073 => return Err(MachineTrap::Breakpoint { pc }),
                _ => return Err(illegal(pc, instruction)),
            },
            _ => return Err(illegal(pc, instruction)),
        }

        self.cpu.pc = next_pc;
        self.cpu.registers[0] = 0;
        Ok(halt)
    }

    fn execute_op_imm(
        &mut self,
        instruction: u32,
        rd: usize,
        rs1: usize,
        funct3: u32,
        pc: u64,
    ) -> Result<(), MachineTrap> {
        let lhs = self.cpu.read(rs1);
        let immediate = immediate_i(instruction);
        let value = match funct3 {
            0 => lhs.wrapping_add(immediate),
            2 => u64::from((lhs as i64) < (immediate as i64)),
            3 => u64::from(lhs < immediate),
            4 => lhs ^ immediate,
            6 => lhs | immediate,
            7 => lhs & immediate,
            1 if instruction >> 26 == 0 => lhs.wrapping_shl((instruction >> 20) & 0x3f),
            5 if instruction >> 26 == 0 => lhs.wrapping_shr((instruction >> 20) & 0x3f),
            5 if instruction >> 26 == 0x10 => ((lhs as i64) >> ((instruction >> 20) & 0x3f)) as u64,
            _ => return Err(illegal(pc, instruction)),
        };
        self.cpu.write(rd, value);
        Ok(())
    }

    fn execute_op_imm_32(
        &mut self,
        instruction: u32,
        rd: usize,
        rs1: usize,
        funct3: u32,
        pc: u64,
    ) -> Result<(), MachineTrap> {
        let lhs = self.cpu.read(rs1) as u32;
        let value = match funct3 {
            0 => lhs.wrapping_add(immediate_i(instruction) as u32),
            1 if instruction >> 25 == 0 => lhs.wrapping_shl((instruction >> 20) & 0x1f),
            5 if instruction >> 25 == 0 => lhs.wrapping_shr((instruction >> 20) & 0x1f),
            5 if instruction >> 25 == 0x20 => ((lhs as i32) >> ((instruction >> 20) & 0x1f)) as u32,
            _ => return Err(illegal(pc, instruction)),
        };
        self.cpu.write(rd, sign_extend(u64::from(value), 32));
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn execute_op(
        &mut self,
        instruction: u32,
        rd: usize,
        rs1: usize,
        rs2: usize,
        funct3: u32,
        funct7: u32,
        pc: u64,
    ) -> Result<(), MachineTrap> {
        let lhs = self.cpu.read(rs1);
        let rhs = self.cpu.read(rs2);
        let value = match (funct7, funct3) {
            (0x00, 0) => lhs.wrapping_add(rhs),
            (0x20, 0) => lhs.wrapping_sub(rhs),
            (0x00, 1) => lhs.wrapping_shl((rhs & 0x3f) as u32),
            (0x00, 2) => u64::from((lhs as i64) < (rhs as i64)),
            (0x00, 3) => u64::from(lhs < rhs),
            (0x00, 4) => lhs ^ rhs,
            (0x00, 5) => lhs.wrapping_shr((rhs & 0x3f) as u32),
            (0x20, 5) => ((lhs as i64) >> (rhs & 0x3f)) as u64,
            (0x00, 6) => lhs | rhs,
            (0x00, 7) => lhs & rhs,
            _ => return Err(illegal(pc, instruction)),
        };
        self.cpu.write(rd, value);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn execute_op_32(
        &mut self,
        instruction: u32,
        rd: usize,
        rs1: usize,
        rs2: usize,
        funct3: u32,
        funct7: u32,
        pc: u64,
    ) -> Result<(), MachineTrap> {
        let lhs = self.cpu.read(rs1) as u32;
        let rhs = self.cpu.read(rs2) as u32;
        let value = match (funct7, funct3) {
            (0x00, 0) => lhs.wrapping_add(rhs),
            (0x20, 0) => lhs.wrapping_sub(rhs),
            (0x00, 1) => lhs.wrapping_shl(rhs & 0x1f),
            (0x00, 5) => lhs.wrapping_shr(rhs & 0x1f),
            (0x20, 5) => ((lhs as i32) >> (rhs & 0x1f)) as u32,
            _ => return Err(illegal(pc, instruction)),
        };
        self.cpu.write(rd, sign_extend(u64::from(value), 32));
        Ok(())
    }
}

fn ensure_aligned(address: u64, bytes: u8, store: bool) -> Result<(), MachineTrap> {
    if address.is_multiple_of(u64::from(bytes)) {
        return Ok(());
    }
    if store {
        Err(MachineTrap::StoreAddressMisaligned { address, bytes })
    } else {
        Err(MachineTrap::LoadAddressMisaligned { address, bytes })
    }
}

fn ensure_instruction_aligned(address: u64) -> Result<(), MachineTrap> {
    if address.is_multiple_of(4) {
        Ok(())
    } else {
        Err(MachineTrap::InstructionAddressMisaligned { address })
    }
}

fn illegal(pc: u64, instruction: u32) -> MachineTrap {
    MachineTrap::IllegalInstruction { pc, instruction }
}

fn sign_extend(value: u64, bits: u32) -> u64 {
    let shift = 64 - bits;
    ((value << shift) as i64 >> shift) as u64
}

fn immediate_i(instruction: u32) -> u64 {
    sign_extend(u64::from(instruction >> 20), 12)
}

fn immediate_s(instruction: u32) -> u64 {
    let value = ((instruction >> 7) & 0x1f) | (((instruction >> 25) & 0x7f) << 5);
    sign_extend(u64::from(value), 12)
}

fn immediate_b(instruction: u32) -> u64 {
    let value = (((instruction >> 8) & 0x0f) << 1)
        | (((instruction >> 25) & 0x3f) << 5)
        | (((instruction >> 7) & 1) << 11)
        | (((instruction >> 31) & 1) << 12);
    sign_extend(u64::from(value), 13)
}

fn immediate_u(instruction: u32) -> u64 {
    sign_extend(u64::from(instruction & 0xffff_f000), 32)
}

fn immediate_j(instruction: u32) -> u64 {
    let value = (((instruction >> 21) & 0x03ff) << 1)
        | (((instruction >> 20) & 1) << 11)
        | (((instruction >> 12) & 0xff) << 12)
        | (((instruction >> 31) & 1) << 20);
    sign_extend(u64::from(value), 21)
}

const fn encode_lui(rd: u32, immediate: u32) -> u32 {
    (immediate << 12) | (rd << 7) | 0x37
}

const fn encode_addi(rd: u32, rs1: u32, immediate: u32) -> u32 {
    ((immediate & 0x0fff) << 20) | (rs1 << 15) | (rd << 7) | 0x13
}

const fn encode_store(rs1: u32, rs2: u32, immediate: u32, funct3: u32) -> u32 {
    (((immediate >> 5) & 0x7f) << 25)
        | (rs2 << 20)
        | (rs1 << 15)
        | (funct3 << 12)
        | ((immediate & 0x1f) << 7)
        | 0x23
}

const fn encode_putc(byte: u8) -> [u32; 2] {
    [encode_addi(6, 0, byte as u32), encode_store(5, 6, 0, 0)]
}

const SMOKE_WORDS: [u32; 23] = {
    let a = encode_putc(b'A');
    let o = encode_putc(b'O');
    let s = encode_putc(b'S');
    let space = encode_putc(b' ');
    let r = encode_putc(b'R');
    let v = encode_putc(b'V');
    let six = encode_putc(b'6');
    let four = encode_putc(b'4');
    let newline = encode_putc(b'\n');
    [
        encode_lui(5, 0x1_0000),
        a[0],
        a[1],
        o[0],
        o[1],
        s[0],
        s[1],
        space[0],
        space[1],
        r[0],
        r[1],
        v[0],
        v[1],
        six[0],
        six[1],
        four[0],
        four[1],
        newline[0],
        newline[1],
        encode_lui(7, 0x100),
        encode_lui(8, 0x5),
        encode_addi(8, 8, 0x555),
        encode_store(7, 8, 0, 2),
    ]
};

const fn words_to_smoke_bytes(words: [u32; 23]) -> [u8; 92] {
    let mut bytes = [0_u8; 92];
    let mut word = 0;
    while word < words.len() {
        let encoded = words[word].to_le_bytes();
        bytes[word * 4] = encoded[0];
        bytes[word * 4 + 1] = encoded[1];
        bytes[word * 4 + 2] = encoded[2];
        bytes[word * 4 + 3] = encoded[3];
        word += 1;
    }
    bytes
}

/// Auditable RV64I probe that prints `AOS RV64` and halts through the standard
/// finisher. It proves the ISA/device/scheduling path without claiming Linux.
pub const RV64_SMOKE_PROGRAM: [u8; 92] = words_to_smoke_bytes(SMOKE_WORDS);

#[cfg(test)]
mod tests {
    use super::*;

    fn machine(console_bytes: usize) -> Machine {
        Machine::new(MachineConfig {
            ram_bytes: 4096,
            max_console_bytes: console_bytes,
        })
        .expect("valid test machine")
    }

    fn words(words: &[u32]) -> Vec<u8> {
        words.iter().flat_map(|word| word.to_le_bytes()).collect()
    }

    #[test]
    fn smoke_program_runs_in_slices_and_halts_exactly() {
        let mut machine = machine(64);
        machine
            .load_program(&RV64_SMOKE_PROGRAM)
            .expect("load smoke program");

        let first = machine.run_slice(3);
        assert_eq!(first.outcome, SliceOutcome::Yielded);
        assert_eq!(first.instructions_retired, 3);
        assert_eq!(first.console, b"A");

        let final_report = machine.run_slice(64);
        assert_eq!(
            final_report.outcome,
            SliceOutcome::Halted(HaltStatus {
                passed: true,
                code: 0,
            })
        );
        assert_eq!(final_report.console, b"OS RV64\n");
        assert_eq!(final_report.total_instructions_retired, 23);

        let repeated = machine.run_slice(64);
        assert_eq!(repeated.outcome, final_report.outcome);
        assert_eq!(repeated.instructions_retired, 0);
        assert!(repeated.console.is_empty());
    }

    #[test]
    fn zero_register_cannot_be_modified() {
        let mut machine = machine(8);
        let program = words(&[encode_addi(0, 0, 42), 0x0010_0073]);
        machine.load_program(&program).expect("load program");

        let report = machine.run_slice(2);
        assert_eq!(machine.register(0), Some(0));
        assert_eq!(
            report.outcome,
            SliceOutcome::Trapped(MachineTrap::Breakpoint { pc: DRAM_BASE + 4 })
        );
        assert_eq!(report.instructions_retired, 1);
    }

    #[test]
    fn invalid_instruction_traps_without_retiring() {
        let mut machine = machine(8);
        machine
            .load_program(&0xffff_ffff_u32.to_le_bytes())
            .expect("load program");

        let report = machine.run_slice(1);
        assert_eq!(
            report.outcome,
            SliceOutcome::Trapped(MachineTrap::IllegalInstruction {
                pc: DRAM_BASE,
                instruction: 0xffff_ffff,
            })
        );
        assert_eq!(report.instructions_retired, 0);
    }

    #[test]
    fn misaligned_jump_traps_without_writing_the_link_register() {
        let mut machine = machine(8);
        // jal x1, +2. The RV64I profile has IALIGN=32, so the jump itself
        // traps and must not commit x1 before control returns to the Realm.
        machine
            .load_program(&0x0020_00ef_u32.to_le_bytes())
            .expect("load program");

        let report = machine.run_slice(1);
        assert_eq!(
            report.outcome,
            SliceOutcome::Trapped(MachineTrap::InstructionAddressMisaligned {
                address: DRAM_BASE + 2,
            })
        );
        assert_eq!(report.instructions_retired, 0);
        assert_eq!(machine.register(1), Some(0));
        assert_eq!(machine.pc(), DRAM_BASE);
    }

    #[test]
    fn console_limit_is_a_guest_trap_not_an_outer_allocation() {
        let mut machine = machine(1);
        machine
            .load_program(&RV64_SMOKE_PROGRAM)
            .expect("load smoke program");

        let report = machine.run_slice(16);
        assert_eq!(
            report.outcome,
            SliceOutcome::Trapped(MachineTrap::ConsoleLimit { limit: 1 })
        );
        assert_eq!(report.console, b"A");
        assert_eq!(report.instructions_retired, 4);
    }

    #[test]
    fn misaligned_store_is_rejected_before_memory_access() {
        let mut machine = machine(8);
        let program = words(&[
            (5 << 7) | 0x17, // auipc x5, 0: current DRAM address
            encode_addi(5, 5, 1),
            encode_addi(6, 0, 42),
            encode_store(5, 6, 0, 2),
        ]);
        machine.load_program(&program).expect("load program");

        let report = machine.run_slice(4);
        assert_eq!(
            report.outcome,
            SliceOutcome::Trapped(MachineTrap::StoreAddressMisaligned {
                address: DRAM_BASE + 1,
                bytes: 4,
            })
        );
        assert_eq!(report.instructions_retired, 3);
    }

    #[test]
    fn image_and_resource_admission_fail_closed() {
        assert_eq!(
            Machine::new(MachineConfig {
                ram_bytes: 4095,
                max_console_bytes: 1,
            })
            .expect_err("unaligned RAM must fail"),
            MachineError::InvalidRamBytes(4095)
        );
        let mut machine = machine(8);
        assert_eq!(
            machine.load_program(&[]),
            Err(MachineError::InvalidProgramBytes {
                image: 0,
                ram: 4096,
            })
        );
    }

    #[test]
    fn console_input_is_read_through_uart_registers() {
        let mut machine = machine(8);
        let program = words(&[
            encode_lui(5, 0x1_0000),
            0x0052_c303, // lbu x6, 5(x5): UART line status
            0x0002_c383, // lbu x7, 0(x5): UART receive byte
            0x0010_0073,
        ]);
        machine.load_program(&program).expect("load program");
        machine.push_console_input(b"Z");

        let report = machine.run_slice(4);
        assert_eq!(machine.register(6), Some(0x61));
        assert_eq!(machine.register(7), Some(u64::from(b'Z')));
        assert!(matches!(report.outcome, SliceOutcome::Trapped(_)));
    }
}
