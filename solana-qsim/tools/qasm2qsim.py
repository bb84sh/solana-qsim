"""Convert a quantum circuit to solana-qsim bytes, so the same circuit runs on a QPU and on-chain.

  python qasm2qsim.py <source> [--out circuit.json] [--check]

<source>: an OpenQASM 2 file, or a built-in circuit:
  bell                 2-qubit Bell pair
  ghz:N                N-qubit GHZ state
  random:N:LAYERS:SEED layers of random U rotations on every qubit, then a CX ladder

Output (bb84-qsim-circuit-v1 JSON): qubit count, gate count, the encoded bytes (hex), the
on-chain circuit hash (sha256 chain over the gate bytes), and OpenQASM 2 of the measured circuit
to send to a QPU. Unsupported gates are first decomposed with Qiskit's transpiler (no
optimisation). --check compares solana-qsim's probabilities with Qiskit's Statevector.

Run with a Python that has qiskit installed.
"""

import argparse
import hashlib
import json
import math
import random
import struct
import subprocess
import sys
from pathlib import Path

from qiskit import QuantumCircuit, qasm2, transpile
from qiskit.quantum_info import Statevector

ROOT = Path(__file__).resolve().parents[2]  # repository root
PROBS_BIN = ROOT / "target" / "release" / "examples" / "probs"

# Gate name -> (opcode, number of qubits, number of angle parameters); see solana-qsim/src/gate.rs.
OPS = {
    "h": (0x00, 1, 0), "x": (0x01, 1, 0), "y": (0x02, 1, 0), "z": (0x03, 1, 0),
    "s": (0x04, 1, 0), "sdg": (0x05, 1, 0), "t": (0x06, 1, 0), "tdg": (0x07, 1, 0),
    "sx": (0x08, 1, 0), "rx": (0x10, 1, 1), "ry": (0x11, 1, 1), "rz": (0x12, 1, 1),
    "p": (0x13, 1, 1), "u1": (0x13, 1, 1), "u": (0x14, 1, 3), "u3": (0x14, 1, 3),
    "cx": (0x20, 2, 0), "cz": (0x21, 2, 0), "swap": (0x22, 2, 0), "cp": (0x23, 2, 1),
    "ccx": (0x30, 3, 0),
}
IGNORED = {"measure", "barrier", "id", "delay"}
MAX_QUBITS = 20


def turns(theta: float) -> int:
    """Angle in radians -> u32 turns (2^32 = 2π), rounded."""
    return round(float(theta) / (2 * math.pi) * 2**32) % 2**32


def encode(circuit: QuantumCircuit) -> tuple[bytes, list[bytes]]:
    """Returns the circuit bytes and the bytes of each gate."""
    if any(inst.operation.name not in OPS and inst.operation.name not in IGNORED for inst in circuit.data):
        plain = circuit.copy_empty_like()
        for inst in circuit.data:
            if inst.operation.name != "barrier":
                plain.append(inst)
        circuit = transpile(plain, basis_gates=sorted(set(OPS) | {"measure"}), optimization_level=0)
    gates = []
    for inst in circuit.data:
        op = inst.operation
        if op.name in IGNORED:
            continue
        if getattr(op, "condition", None) is not None:
            raise ValueError(f"classically controlled {op.name} is not supported")
        if op.name not in OPS:
            raise ValueError(f"unsupported gate {op.name}")
        code, nq, na = OPS[op.name]
        qubits = [circuit.find_bit(q).index for q in inst.qubits]
        if len(qubits) != nq or len(op.params) != na:
            raise ValueError(f"unexpected arity for {op.name}")
        angles = [turns(p) for p in op.params]
        gates.append(bytes([code, *qubits]) + b"".join(struct.pack("<I", a) for a in angles))
    return b"".join(gates), gates


def circuit_hash(gates: list[bytes]) -> str:
    h = bytes(32)
    for g in gates:
        h = hashlib.sha256(h + g).digest()
    return h.hex()


def builtin(spec: str) -> QuantumCircuit:
    name, *args = spec.split(":")
    if name == "bell":
        return builtin("ghz:2")
    if name == "ghz":
        n = int(args[0])
        qc = QuantumCircuit(n, name=f"ghz{n}")
        qc.h(0)
        for q in range(1, n):
            qc.cx(q - 1, q)
        return qc
    if name == "random":
        n, layers, seed = int(args[0]), int(args[1]), int(args[2])
        rng = random.Random(seed)
        qc = QuantumCircuit(n, name=f"random{n}x{layers}s{seed}")
        for layer in range(layers):
            for q in range(n):
                qc.u(*(rng.uniform(0, 2 * math.pi) for _ in range(3)), q)
            for q in range(layer % 2, n - 1, 2):
                qc.cx(q, q + 1)
        return qc
    raise ValueError(f"unknown built-in circuit {spec!r}")


def load(source: str) -> QuantumCircuit:
    path = Path(source)
    if path.suffix == ".qasm" and path.exists():
        return qasm2.load(str(path), custom_instructions=qasm2.LEGACY_CUSTOM_INSTRUCTIONS)
    return builtin(source)


def check(circuit: QuantumCircuit, data: bytes) -> float:
    """Max |p_qsim - p_qiskit| over all outcomes."""
    if not PROBS_BIN.exists():
        sys.exit(f"build the checker first: cargo build -p solana-qsim --release --example probs")
    out = subprocess.run([str(PROBS_BIN), str(circuit.num_qubits), data.hex()], capture_output=True, text=True, check=True)
    ours = json.loads(out.stdout)["probabilities"]
    theirs = Statevector.from_instruction(circuit.remove_final_measurements(inplace=False)).probabilities()
    return max(abs(a - b) for a, b in zip(ours, theirs))


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("source")
    ap.add_argument("--out", help="write the circuit JSON here (default: print it)")
    ap.add_argument("--check", action="store_true", help="compare probabilities with Qiskit's Statevector")
    args = ap.parse_args()

    circuit = load(args.source)
    if circuit.num_qubits > MAX_QUBITS:
        sys.exit(f"{circuit.num_qubits} qubits; solana-qsim supports up to {MAX_QUBITS}")
    data, gates = encode(circuit)
    measured = circuit.remove_final_measurements(inplace=False)
    measured.measure_all()
    result = {
        "format": "bb84-qsim-circuit-v1",
        "name": circuit.name,
        "source": args.source,
        "num_qubits": circuit.num_qubits,
        "gates": len(gates),
        "bytes": len(data),
        "hex": data.hex(),
        "circuit_hash": circuit_hash(gates),
        "qasm": qasm2.dumps(measured),
    }
    if args.check:
        err = check(circuit, data)
        result["check_max_prob_error"] = err
        print(f"check vs Qiskit Statevector: max probability error {err:.3e}", file=sys.stderr)
        if err > 1e-6:
            sys.exit("check failed")
    text = json.dumps(result, indent=2)
    if args.out:
        Path(args.out).write_text(text + "\n")
        print(f"wrote {args.out}: {result['num_qubits']} qubits, {result['gates']} gates, {result['bytes']} bytes, hash {result['circuit_hash'][:16]}…", file=sys.stderr)
    else:
        print(text)


if __name__ == "__main__":
    main()
