//! Gates and their compact byte encoding: one opcode byte, then qubit bytes, then angles as
//! little-endian `u32` turns (θ = 2π · turns / 2^32). A circuit is a concatenation of gates.

use crate::Error;

/// Rotation angles are in turns: `u32::MAX + 1` = one full turn (2π), so `1 << 30` = π/2.
pub type Turns = u32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gate {
    H(u8),
    X(u8),
    Y(u8),
    Z(u8),
    S(u8),
    Sdg(u8),
    T(u8),
    Tdg(u8),
    /// √X
    Sx(u8),
    Rx(u8, Turns),
    Ry(u8, Turns),
    Rz(u8, Turns),
    /// Phase gate diag(1, e^{iλ}) (OpenQASM `p` / `u1`).
    P(u8, Turns),
    /// OpenQASM `u(θ, φ, λ)`.
    U(u8, Turns, Turns, Turns),
    /// Control, target.
    Cx(u8, u8),
    Cz(u8, u8),
    Swap(u8, u8),
    /// Controlled phase diag(1, 1, 1, e^{iλ}).
    Cp(u8, u8, Turns),
    /// Toffoli: control, control, target.
    Ccx(u8, u8, u8),
}

pub mod op {
    pub const H: u8 = 0x00;
    pub const X: u8 = 0x01;
    pub const Y: u8 = 0x02;
    pub const Z: u8 = 0x03;
    pub const S: u8 = 0x04;
    pub const SDG: u8 = 0x05;
    pub const T: u8 = 0x06;
    pub const TDG: u8 = 0x07;
    pub const SX: u8 = 0x08;
    pub const RX: u8 = 0x10;
    pub const RY: u8 = 0x11;
    pub const RZ: u8 = 0x12;
    pub const P: u8 = 0x13;
    pub const U: u8 = 0x14;
    pub const CX: u8 = 0x20;
    pub const CZ: u8 = 0x21;
    pub const SWAP: u8 = 0x22;
    pub const CP: u8 = 0x23;
    pub const CCX: u8 = 0x30;
}

impl Gate {
    /// The qubits the gate acts on (unused slots are `u8::MAX`), and how many.
    pub fn qubits(&self) -> ([u8; 3], usize) {
        use Gate::*;
        match *self {
            H(q) | X(q) | Y(q) | Z(q) | S(q) | Sdg(q) | T(q) | Tdg(q) | Sx(q) | Rx(q, _) | Ry(q, _) | Rz(q, _) | P(q, _)
            | U(q, ..) => ([q, u8::MAX, u8::MAX], 1),
            Cx(a, b) | Cz(a, b) | Swap(a, b) | Cp(a, b, _) => ([a, b, u8::MAX], 2),
            Ccx(a, b, c) => ([a, b, c], 3),
        }
    }

    /// Checks qubit indices against an `n`-qubit register.
    pub fn validate(&self, n: u32) -> Result<(), Error> {
        let (q, k) = self.qubits();
        for i in 0..k {
            if q[i] as u32 >= n {
                return Err(Error::QubitOutOfRange(q[i]));
            }
            if q[..i].contains(&q[i]) {
                return Err(Error::RepeatedQubit(q[i]));
            }
        }
        Ok(())
    }

    /// Bytes this gate takes in a circuit.
    pub fn encoded_len(&self) -> usize {
        let (_, k) = self.qubits();
        let angles = match self {
            Gate::Rx(..) | Gate::Ry(..) | Gate::Rz(..) | Gate::P(..) | Gate::Cp(..) => 1,
            Gate::U(..) => 3,
            _ => 0,
        };
        1 + k + 4 * angles
    }

    /// Writes the gate into `out` (at least `encoded_len()` bytes); returns the bytes written.
    pub fn encode(&self, out: &mut [u8]) -> usize {
        use Gate::*;
        let (code, angles): (u8, [Turns; 3]) = match *self {
            H(_) => (op::H, [0; 3]),
            X(_) => (op::X, [0; 3]),
            Y(_) => (op::Y, [0; 3]),
            Z(_) => (op::Z, [0; 3]),
            S(_) => (op::S, [0; 3]),
            Sdg(_) => (op::SDG, [0; 3]),
            T(_) => (op::T, [0; 3]),
            Tdg(_) => (op::TDG, [0; 3]),
            Sx(_) => (op::SX, [0; 3]),
            Rx(_, a) => (op::RX, [a, 0, 0]),
            Ry(_, a) => (op::RY, [a, 0, 0]),
            Rz(_, a) => (op::RZ, [a, 0, 0]),
            P(_, a) => (op::P, [a, 0, 0]),
            U(_, a, b, c) => (op::U, [a, b, c]),
            Cx(..) => (op::CX, [0; 3]),
            Cz(..) => (op::CZ, [0; 3]),
            Swap(..) => (op::SWAP, [0; 3]),
            Cp(_, _, a) => (op::CP, [a, 0, 0]),
            Ccx(..) => (op::CCX, [0; 3]),
        };
        let (q, k) = self.qubits();
        out[0] = code;
        out[1..1 + k].copy_from_slice(&q[..k]);
        let mut at = 1 + k;
        for a in angles.iter().take((self.encoded_len() - at) / 4) {
            out[at..at + 4].copy_from_slice(&a.to_le_bytes());
            at += 4;
        }
        at
    }

    /// Reads one gate from the front of `bytes`; returns it and the bytes it used.
    pub fn decode(bytes: &[u8]) -> Result<(Gate, usize), Error> {
        let code = *bytes.first().ok_or(Error::Truncated)?;
        let q = |i: usize| bytes.get(i).copied().ok_or(Error::Truncated);
        let a = |i: usize| -> Result<Turns, Error> {
            let b = bytes.get(i..i + 4).ok_or(Error::Truncated)?;
            Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        };
        let gate = match code {
            op::H => Gate::H(q(1)?),
            op::X => Gate::X(q(1)?),
            op::Y => Gate::Y(q(1)?),
            op::Z => Gate::Z(q(1)?),
            op::S => Gate::S(q(1)?),
            op::SDG => Gate::Sdg(q(1)?),
            op::T => Gate::T(q(1)?),
            op::TDG => Gate::Tdg(q(1)?),
            op::SX => Gate::Sx(q(1)?),
            op::RX => Gate::Rx(q(1)?, a(2)?),
            op::RY => Gate::Ry(q(1)?, a(2)?),
            op::RZ => Gate::Rz(q(1)?, a(2)?),
            op::P => Gate::P(q(1)?, a(2)?),
            op::U => Gate::U(q(1)?, a(2)?, a(6)?, a(10)?),
            op::CX => Gate::Cx(q(1)?, q(2)?),
            op::CZ => Gate::Cz(q(1)?, q(2)?),
            op::SWAP => Gate::Swap(q(1)?, q(2)?),
            op::CP => Gate::Cp(q(1)?, q(2)?, a(3)?),
            op::CCX => Gate::Ccx(q(1)?, q(2)?, q(3)?),
            other => return Err(Error::UnknownGate(other)),
        };
        Ok((gate, gate.encoded_len()))
    }
}

/// A circuit as encoded bytes; iterate to decode gate by gate.
#[derive(Clone, Copy, Debug)]
pub struct Circuit<'a>(pub &'a [u8]);

impl<'a> Iterator for Circuit<'a> {
    type Item = Result<Gate, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.0.is_empty() {
            return None;
        }
        match Gate::decode(self.0) {
            Ok((gate, used)) => {
                self.0 = &self.0[used..];
                Some(Ok(gate))
            }
            Err(e) => {
                self.0 = &[];
                Some(Err(e))
            }
        }
    }
}
