# solana-qsim (crate)

Fixed-point quantum circuit simulator for Solana programs: `no_std`, no floats, no allocation. The state is any `&mut [Complex]`, on the heap or in account data.

```rust
use solana_qsim::{Complex, Gate, State, Count, score};

let mut amps = vec![Complex::ZERO; 1 << 2];
let mut state = State::new(&mut amps)?;
state.reset();                                   // |00⟩
state.apply(&Gate::H(0))?;
state.apply(&Gate::Cx(0, 1))?;                   // Bell pair
let s = score(&state, &[Count { outcome: 0, count: 478 }, Count { outcome: 3, count: 499 }])?;
// s.fidelity_ppm, s.tvd_ppm, s.xeb_ppm
```

- `State::apply_range(gate, start, end)` runs part of a gate, so large registers can be split across transactions.
- `Circuit(&bytes)` iterates the compact gate encoding (see the repository README).
- Accuracy tests against an f64 reference are in [`tests/reference.rs`](tests/reference.rs).

See the [repository README](../README.md) for the program, the tools and the devnet results.
