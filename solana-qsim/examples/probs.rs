//! Runs an encoded circuit and prints its outcome probabilities as JSON (for cross-checks).
//!
//!   cargo run -p solana-qsim --release --example probs -- <num_qubits> <circuit-hex>

use solana_qsim::{fixed::ONE, Circuit, Complex, State};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (n, hex) = match args.as_slice() {
        [n, hex] => (n.parse::<u32>().expect("num_qubits"), hex.as_str()),
        _ => {
            eprintln!("usage: probs <num_qubits> <circuit-hex>");
            std::process::exit(1)
        }
    };
    let bytes: Vec<u8> = (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("hex")).collect();
    let mut amps = vec![Complex::ZERO; 1 << n];
    let mut state = State::new(&mut amps).expect("register size");
    state.reset();
    let gates = state.run(Circuit(&bytes)).unwrap_or_else(|e| panic!("circuit: {e:?}"));
    let one = ONE as f64;
    let probs: Vec<String> = amps.iter().map(|a| format!("{:.12}", (a.re as f64 / one).powi(2) + (a.im as f64 / one).powi(2))).collect();
    println!("{{\"gates\":{gates},\"probabilities\":[{}]}}", probs.join(","));
}
