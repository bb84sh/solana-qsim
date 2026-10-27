"""Measure a qsim circuit and write the counts for on-chain scoring.

Local (free):
  python sample.py circuit.json --out counts.json [--shots 1024] [--mode ideal|noisy|uniform] [--seed N]
    ideal    exact samples from Qiskit's StatevectorSampler
    noisy    ideal samples, then each measured bit flips with probability --flip (default 0.02)
    uniform  uniformly random bitstrings: a "QPU result" that ignored the circuit (should fail)

Amazon Braket QPU (paid; Rigetti, IQM, IonQ):
  python sample.py circuit.json --backend braket --preflight [--shots 1024]       # status + cost, submits nothing
  python sample.py circuit.json --backend braket --device Cepheus-1-108Q --confirm-qpu --submit-only --out counts.json
  python sample.py --fetch counts.json.pending.json                               # waits for the task, writes counts.json

Output: bb84-qsim-counts-v1 JSON with Qiskit-style bitstrings (leftmost = highest qubit),
the job id, source (LocalSimulator / Qpu) and result_hash = sha256 of the canonical counts
("bits:count" lines, sorted), which a qproof receipt records on-chain.
Run with a Python that has qiskit (and amazon-braket-sdk for --backend braket).
"""

import argparse
import datetime
import hashlib
import json
import math
import os
import random
import struct
import time
from collections import Counter
from pathlib import Path

PER_TASK_USD = 0.30
PER_SHOT_USD = {"IonQ": 0.08, "Rigetti": 0.0009, "IQM": 0.00145}  # check aws.amazon.com/braket/pricing


def result_hash(counts: dict) -> str:
    canonical = "\n".join(f"{bits}:{c}" for bits, c in sorted(counts.items()))
    return hashlib.sha256(canonical.encode()).hexdigest()


def write_counts(path, spec, counts, *, backend, source, job_id, shots, extra=None):
    counts = dict(sorted(counts.items()))
    out = {
        "format": "bb84-qsim-counts-v1",
        "circuit_hash": spec["circuit_hash"],
        "num_qubits": spec["num_qubits"],
        "backend": backend,
        "source": source,
        "job_id": job_id,
        "shots": shots,
        "measured_at": datetime.datetime.now(datetime.timezone.utc).isoformat(timespec="seconds"),
        "counts": counts,
        "result_hash": result_hash(counts),
        **(extra or {}),
    }
    Path(path).write_text(json.dumps(out, indent=2) + "\n")
    print(f"wrote {path}: {backend} ({source}), {sum(counts.values())} shots, {len(counts)} distinct outcomes, result_hash {out['result_hash'][:16]}…")


# ---- local ----

def sample_local(spec, args):
    from qiskit import qasm2
    from qiskit.primitives import StatevectorSampler

    n = spec["num_qubits"]
    rng = random.Random(args.seed)
    if args.mode == "uniform":
        return Counter(format(rng.getrandbits(n), f"0{n}b") for _ in range(args.shots))
    qc = qasm2.loads(spec["qasm"], custom_instructions=qasm2.LEGACY_CUSTOM_INSTRUCTIONS)
    counts = Counter(StatevectorSampler(seed=args.seed).run([qc], shots=args.shots).result()[0].data.meas.get_counts())
    if args.mode == "noisy":
        flipped = Counter()
        for bits, c in counts.items():
            for _ in range(c):
                flipped["".join(b if rng.random() >= args.flip else "10"[int(b)] for b in bits)] += 1
        counts = flipped
    return counts


# ---- Amazon Braket ----

def braket_circuit(spec):
    """Rebuilds the circuit from its qsim bytes, gate for gate, as a Braket Circuit."""
    from braket.circuits import Circuit

    data = bytes.fromhex(spec["hex"])
    rad = lambda b: struct.unpack("<I", b)[0] / 2**32 * 2 * math.pi
    qc, i = Circuit(), 0
    one = {0x00: "h", 0x01: "x", 0x02: "y", 0x03: "z", 0x04: "s", 0x05: "si", 0x06: "t", 0x07: "ti", 0x08: "v"}
    rot = {0x10: "rx", 0x11: "ry", 0x12: "rz", 0x13: "phaseshift"}
    two = {0x20: "cnot", 0x21: "cz", 0x22: "swap"}
    while i < len(data):
        op = data[i]
        if op in one:
            getattr(qc, one[op])(data[i + 1]); i += 2
        elif op in rot:
            getattr(qc, rot[op])(data[i + 1], rad(data[i + 2:i + 6])); i += 6
        elif op == 0x14:
            qc.u(data[i + 1], rad(data[i + 2:i + 6]), rad(data[i + 6:i + 10]), rad(data[i + 10:i + 14])); i += 14
        elif op in two:
            getattr(qc, two[op])(data[i + 1], data[i + 2]); i += 3
        elif op == 0x23:
            qc.cphaseshift(data[i + 1], data[i + 2], rad(data[i + 3:i + 7])); i += 7
        elif op == 0x30:
            qc.ccnot(data[i + 1], data[i + 2], data[i + 3]); i += 4
        else:
            raise SystemExit(f"unknown opcode {op:#x}")
    # Measure every qubit of the register, even ones no gate touched.
    for q in range(spec["num_qubits"]):
        if q not in qc.qubits:
            qc.i(q)
    return qc


def gate_qpus():
    from braket.aws import AwsDevice

    return [d for d in AwsDevice.get_devices(types=["QPU"]) if "braket.ir.openqasm.program" in d.properties.action]


def find_device(name):
    for d in gate_qpus():
        if d.name.lower() == name.lower():
            return d
    raise SystemExit(f"No gate-model Braket QPU named {name!r}; run --preflight to list them.")


def estimate_usd(provider, shots):
    per_shot = PER_SHOT_USD.get(provider)
    return None if per_shot is None else PER_TASK_USD + per_shot * shots


def preflight(spec, shots):
    for d in gate_qpus():
        n = d.properties.paradigm.qubitCount
        cost = estimate_usd(d.provider_name, shots)
        try:
            queue = d.queue_depth().quantum_tasks
            queue_text = f", queue {queue}"
        except Exception:
            queue_text = ""
        fits = "ok" if n >= spec["num_qubits"] else f"too small for {spec['num_qubits']} qubits"
        print(f"  {d.provider_name} {d.name}: {d.status}{queue_text}, {n} qubits, {fits}, {shots} shots "
              f"{f'~${cost:.2f}' if cost is not None else 'price unknown'}")
    print("No task was submitted.")


def to_qiskit_order(bits, measured_qubits, n):
    """Braket strings list qubits in measured_qubits order; Qiskit puts qubit 0 rightmost."""
    pos = {q: k for k, q in enumerate(measured_qubits)}
    return "".join(bits[pos[q]] for q in reversed(range(n)))


def fetch(pending_path, timeout):
    from braket.aws import AwsQuantumTask

    pending = json.loads(Path(pending_path).read_text())
    spec = json.loads(Path(pending["circuit"]).read_text())
    task = AwsQuantumTask(pending["job_id"])
    print(f"waiting for task {task.id} up to {timeout}s (queue time included)", flush=True)
    started = time.time()
    while (state := task.state()) not in ("COMPLETED", "FAILED", "CANCELLED"):
        if time.time() - started > timeout:
            raise SystemExit(f"still {state} after {timeout}s; fetch again later (task not cancelled)")
        time.sleep(15)
    if state != "COMPLETED":
        raise SystemExit(f"task ended as {state}; no results")
    result = task.result()
    n = spec["num_qubits"]
    counts = Counter()
    for bits, c in result.measurement_counts.items():
        counts[to_qiskit_order(bits, result.measured_qubits, n)] += c
    write_counts(pending["out"], spec, counts, backend=pending["backend"], source="Qpu", job_id=task.id,
                 shots=pending["shots"], extra={"hardware": {"device_arn": pending["device_arn"],
                                                             "estimated_usd": pending["estimated_usd"]}})


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("circuit", nargs="?")
    ap.add_argument("--out")
    ap.add_argument("--shots", type=int, default=1024)
    ap.add_argument("--mode", choices=["ideal", "noisy", "uniform"], default="ideal")
    ap.add_argument("--flip", type=float, default=0.02)
    ap.add_argument("--seed", type=int, default=None)
    ap.add_argument("--backend", choices=["local", "braket"], default="local")
    ap.add_argument("--device")
    ap.add_argument("--preflight", action="store_true", help="braket: list devices and cost, submit nothing")
    ap.add_argument("--confirm-qpu", action="store_true", help="braket: required to submit a paid task")
    ap.add_argument("--submit-only", action="store_true", help="braket: submit, save <out>.pending.json, exit")
    ap.add_argument("--fetch", metavar="PENDING_JSON")
    ap.add_argument("--timeout", type=int, default=3600)
    args = ap.parse_args()

    if args.fetch:
        return fetch(args.fetch, args.timeout)
    if not args.circuit:
        ap.error("circuit.json is required")
    spec = json.loads(Path(args.circuit).read_text())

    if args.backend == "local":
        if not args.out:
            ap.error("--out is required")
        counts = sample_local(spec, args)
        backend = {"ideal": "local:statevector_sampler", "noisy": f"local:statevector_sampler+flip{args.flip}",
                   "uniform": "local:uniform_random"}[args.mode]
        return write_counts(args.out, spec, counts, backend=backend, source="LocalSimulator",
                            job_id=f"local-{args.mode}-{int(time.time())}", shots=args.shots)

    if args.preflight:
        return preflight(spec, args.shots)
    if not (args.device and args.out):
        ap.error("--device and --out are required for a Braket run")
    if not args.confirm_qpu:
        ap.error("real QPU tasks are billed: run --preflight first, then add --confirm-qpu")
    device = find_device(args.device)
    if device.status != "ONLINE":
        raise SystemExit(f"{device.name} is {device.status}; pick another device")
    cost = estimate_usd(device.provider_name, args.shots)
    qc = braket_circuit(spec)
    task = device.run(qc, shots=args.shots)
    print(f"submitted task {task.id} on {device.provider_name} {device.name} (~${cost:.2f})", flush=True)
    pending = {"job_id": task.id, "backend": f"braket:{device.name}", "device_arn": device.arn,
               "circuit": str(Path(args.circuit).resolve()), "shots": args.shots, "out": args.out, "estimated_usd": cost}
    pending_path = f"{args.out}.pending.json"
    os.makedirs(os.path.dirname(pending_path) or ".", exist_ok=True)
    Path(pending_path).write_text(json.dumps(pending, indent=2) + "\n")
    print(f"saved {pending_path}")
    if args.submit_only:
        print(f"fetch later with: python sample.py --fetch {pending_path}")
        return
    fetch(pending_path, args.timeout)


if __name__ == "__main__":
    main()
