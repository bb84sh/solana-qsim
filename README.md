# solana-qsim

**Quantum computers run the circuit. Solana checks the answer.**

`solana-qsim` is a quantum circuit simulator that runs inside a Solana program. A program can re-run a small circuit itself, exactly, and score a quantum computer's measured counts against it. Combined with an on-chain receipt of the QPU job, a forged or tampered result is rejected on-chain.

Part of [bb84](https://bb84.sh). Experimental, **devnet only, not audited**.

## Live on Solana devnet

**Programs:**

| Program | Address | What it does |
|---|---|---|
| `qsim` | [`6sHufVFfaMwxPuDKxBuV8qZqNot8bJNgtFKhZKXHdhSw`](https://explorer.solana.com/address/6sHufVFfaMwxPuDKxBuV8qZqNot8bJNgtFKhZKXHdhSw?cluster=devnet) | Simulates circuits and scores counts |
| `qproof` | [`6pxqWGoxSRBwmL6GJie9BD43HGe3ybFu9xmJQ24TvcXB`](https://explorer.solana.com/address/6pxqWGoxSRBwmL6GJie9BD43HGe3ybFu9xmJQ24TvcXB?cluster=devnet) | Receipts for QPU jobs: job id hash, source (Qpu / simulator), backend, circuit hash, results hash |

### A real QPU result, checked on-chain

**The run:**
- A 5-qubit GHZ circuit ([`demo/ghz5.json`](solana-qsim/demo/ghz5.json)), 1,024 shots on **Rigetti Cepheus-1-108Q** through Amazon Braket, on 2026-10-27.
- Counts: [`demo/ghz5-counts-rigetti.json`](solana-qsim/demo/ghz5-counts-rigetti.json).
- 70% of shots landed on the two ideal outcomes (00000: 375, 11111: 341); the rest is hardware noise.

| Step | Result |
|---|---|
| Receipt in `qproof` (source **Qpu**) | [`5LBExjr1NvBP8wzA9XBZc3cZsrQuiKKYhaSAntg8KPCn`](https://explorer.solana.com/address/5LBExjr1NvBP8wzA9XBZc3cZsrQuiKKYhaSAntg8KPCn?cluster=devnet) ([tx](https://explorer.solana.com/tx/4M4zaKqRsVvvHfHETcNUHHUAsYNX93TwHea8H9dDVsH2s2C6taV6giLubjQgbogB4S32prw7MNWaU7hJTCFGUMMA?cluster=devnet)) |
| `simulate_receipt`: the program checks the circuit and counts against the receipt, then scores | **fidelity 0.699 · XEB 10.19 · TVD 0.301**, 26,398 CU ([tx](https://explorer.solana.com/tx/ynsk9TXEjpnakshxc6DhzvE5ZB3eVy6aAVGu8jpHSgEsNy4hW6RqwM9a2bRab1QAnM5EMuvDp1V8C18rAGtcZWk?cluster=devnet)) |
| Forged "better" counts (00000: 512, 11111: 512) sent to the program | ❌ rejected on-chain: `ResultMismatch` (6011) |
| The same counts claimed for another circuit | ❌ `CircuitMismatch` (6010) (test) |

**Reading the score:**

| Counts for GHZ-5 | Fidelity | XEB |
|---|---|---|
| Ideal | 1.00 | 15.0 |
| **Rigetti hardware** | **0.70** | **10.2** |
| Local simulation with 2% bit flips | 0.89 | 13.3 |
| Uniform random noise posing as a result | 0.07 | 0.1 |

The task's AWS ARN is withheld because it contains the account number. The counts file carries its sha256 (`job_id_sha256`), which is how the receipt is keyed, plus the bare task id.

### What is and isn't proven
- ✅ **On-chain:** these exact counts are the ones recorded for that job, they belong to this exact circuit, and they score as shown.
- ❌ **Not proven:** that the job ran on quantum hardware. The relayer posts receipts, and a 5-qubit result can be simulated classically.
- `qsim` itself is a **classical** simulation of quantum circuits. Anything it can simulate, a laptop can too. It verifies small QPU results; it doesn't provide quantum computation or prove quantum advantage.

## Numbers (LiteSVM, SBPF v0)

**One transaction (`simulate`, state on the heap):**

| Circuit | Qubits | Gates | CU |
|---|---|---|---|
| Bell pair + score | 2 | 2 | 5,226 |
| GHZ | 11 | 11 | 151,162 |
| Random layers (U + CX ladder) | 8 | 92 | 818,445 |
| Random layers | 4 | 88 | 115,761 (limited by the 1,232-byte transaction) |

**Across transactions (`apply_range`, state in an account, up to 20 qubits / 8 MiB):**

| | X | T | H | RZ | U | CX | CZ | CCX |
|---|---|---|---|---|---|---|---|---|
| CU per work item | 10.1 | 24.4 | 40.6 | 52.8 | 85.9 | 14.9 | 13.6 | 16.9 |

- **At 20 qubits:** about 16–23 transactions per H gate, 34 per U, 3–8 per CX.
- **Scoring** reads only the observed outcomes: about 5k CU, even at 20 qubits.

**Accuracy:**
- Against an independent f64 simulator: worst amplitude error **7×10⁻⁹** after 80 random gates.
- The converter against Qiskit's `Statevector`: worst probability difference **2×10⁻⁹**.

## How it works

- **Library** ([`solana-qsim/src`](solana-qsim/src)):
  - `no_std`, no floats, deterministic, so on-chain and off-chain results are bit-for-bit identical.
  - Amplitudes are Q1.30 fixed-point complex numbers (8 bytes); sin/cos are computed in fixed point.
  - **Gates:** H, X, Y, Z, S, S†, T, T†, √X, RX, RY, RZ, P, U, CX, CZ, SWAP, CP, CCX, in a compact byte encoding. Angles are `u32` turns (2³² = 2π).
  - **Scores:** Hellinger fidelity (as in Qiskit), total variation distance, linear XEB.
- **Program** ([`programs/qsim`](programs/qsim)):

  | Instruction | What it does |
  |---|---|
  | `simulate` | One transaction, up to 11 qubits; returns the score |
  | `simulate_receipt` | Same, but first checks on-chain that the circuit hash chain and sha256 of the canonical counts equal the `qproof` receipt's |
  | `init_state`, `apply`, `apply_range`, `score` | State in an account, up to 20 qubits; gates can be split across transactions; every gate is added to a sha256 hash chain |

- **Circuit hash:** h ← sha256(h ‖ gate bytes), starting from 32 zero bytes.
- **Counts hash:** sha256 of the sorted `bits:count` lines (Qiskit bit order: qubit 0 rightmost).

## Run it

```bash
# library: tests against an f64 reference
cargo test -p solana-qsim --release -- --nocapture

# programs + LiteSVM tests and benchmarks (Anchor 1.x, Solana CLI)
anchor build --arch v0
cargo test -p qsim --release -- --nocapture --test-threads 1

# tools (Python with qiskit; Node 20+)
pip install qiskit amazon-braket-sdk && npm install
cargo build -p solana-qsim --release --example probs
python solana-qsim/tools/qasm2qsim.py ghz:5 --check --out /tmp/ghz5.json   # circuit → bytes, checked vs Qiskit
python solana-qsim/tools/sample.py /tmp/ghz5.json --mode noisy --out /tmp/counts.json
node solana-qsim/tools/qsim.js simulate /tmp/ghz5.json /tmp/counts.json --dry-run   # score on devnet, free

# verify the Rigetti result yourself (free dry run against the devnet receipt)
node solana-qsim/tools/qsim.js simulate solana-qsim/demo/ghz5.json solana-qsim/demo/ghz5-counts-rigetti.json --dry-run
```

**Tools** ([`solana-qsim/tools`](solana-qsim/tools)):

| Tool | What it does |
|---|---|
| `qasm2qsim.py` | OpenQASM 2 or built-in circuits (`bell`, `ghz:N`, `random:N:L:SEED`) → bytes, circuit hash, and OpenQASM for the QPU; `--check` compares with Qiskit |
| `sample.py` | Counts from a local sampler (`ideal`, `noisy`, `uniform`) or Amazon Braket (`--preflight` is free; paid runs need `--confirm-qpu`) |
| `qsim.js` | `record` (qproof receipt), `simulate` (uses `simulate_receipt` when a receipt exists; `--dry-run` is free), `show` |

## Verify the deployed programs

On 2026-10-27, `anchor build --arch v0` of this repository (Rust 1.89.0, Anchor 1.2.1) produced byte-identical program binaries to the ones deployed on devnet:

| Program | sha256 of the binary |
|---|---|
| `qsim` | `3eeb42bf0a4be81c3bc520b2724f5d575c4ccfe31ef9ca96a3d42bef431f0165` (183,464 bytes) |
| `qproof` | `053cca23920e70dedb64d7a1e081f2708732eedcef2a19d0171914313dfdf8e8` (181,320 bytes) |

```bash
anchor build --arch v0
solana program dump 6sHufVFfaMwxPuDKxBuV8qZqNot8bJNgtFKhZKXHdhSw qsim-devnet.so -u devnet
head -c $(stat -c%s target/deploy/qsim.so) qsim-devnet.so | sha256sum; sha256sum target/deploy/qsim.so
```

This is a local build, not a containerised verifiable build (`solana-verify`); toolchain differences can change the bytes.

## Limits

- 11 qubits per transaction on the heap; 20 qubits in an account, across many transactions.
- One transaction carries the circuit and the counts, which fits GHZ-style results. Results with hundreds of distinct outcomes need a counts buffer (planned).
- Fixed-point rounding is about 2⁻³⁰ per operation, far below QPU noise.
- The programs are upgradeable and unaudited. Devnet only.

## License

MIT. See [LICENSE](LICENSE).
