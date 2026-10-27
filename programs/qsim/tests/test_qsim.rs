//! qsim in LiteSVM: correctness against the host library, rejections, and compute-unit
//! benchmarks (run with `cargo test -p qsim --release -- --nocapture --test-threads 1`).

use {
    anchor_lang::{
        prelude::Pubkey,
        solana_program::instruction::{AccountMeta, Instruction},
        AnchorDeserialize, InstructionData, ToAccountMetas,
    },
    litesvm::LiteSVM,
    qsim::{Count, QsimScore},
    solana_account::Account,
    solana_keypair::Keypair,
    solana_message::{Message, VersionedMessage},
    solana_qsim::{Complex, Gate, State},
    solana_signer::Signer,
    solana_transaction::versioned::VersionedTransaction,
};

const COMPUTE_BUDGET: Pubkey = anchor_lang::pubkey!("ComputeBudget111111111111111111111111111111");
const MAX_CU: u32 = 1_400_000;
const PACKET_DATA_SIZE: usize = 1232;

#[derive(Debug)]
struct Sent {
    units: u64,
    size: usize,
    return_data: Vec<u8>,
}

fn send(svm: &mut LiteSVM, ixs: &[Instruction], payer: &Keypair, signers: &[&Keypair]) -> Result<Sent, String> {
    svm.expire_blockhash();
    let mut all = vec![compute_limit(MAX_CU)];
    all.extend_from_slice(ixs);
    let msg = Message::new_with_blockhash(&all, Some(&payer.pubkey()), &svm.latest_blockhash());
    let message = VersionedMessage::Legacy(msg);
    let size = 1 + 64 * signers.len() + message.serialize().len();
    let tx = VersionedTransaction::try_new(message, signers).unwrap();
    svm.send_transaction(tx)
        .map(|meta| Sent { units: meta.compute_units_consumed, size, return_data: meta.return_data.data })
        .map_err(|failed| format!("{:?} {:?}", failed.err, failed.meta.logs.iter().rev().take(3).collect::<Vec<_>>()))
}

fn compute_limit(units: u32) -> Instruction {
    let mut data = vec![2u8];
    data.extend_from_slice(&units.to_le_bytes());
    Instruction::new_with_bytes(COMPUTE_BUDGET, &data, vec![])
}

fn setup() -> (LiteSVM, Keypair) {
    let mut svm = LiteSVM::new();
    let deploy = concat!(env!("CARGO_TARGET_TMPDIR"), "/../deploy");
    svm.add_program_from_file(qsim::id(), format!("{deploy}/qsim.so")).unwrap();
    svm.add_program_from_file(qproof::id(), format!("{deploy}/qproof.so")).unwrap();
    let payer = Keypair::new();
    svm.airdrop(&payer.pubkey(), 100_000_000_000).unwrap();
    (svm, payer)
}

fn encode(gates: &[Gate]) -> Vec<u8> {
    let mut out = vec![0u8; gates.iter().map(|g| g.encoded_len()).sum()];
    let mut at = 0;
    for g in gates {
        at += g.encode(&mut out[at..]);
    }
    out
}

fn counts(pairs: &[(u32, u32)]) -> Vec<Count> {
    pairs.iter().map(|&(outcome, count)| Count { outcome, count }).collect()
}

fn simulate_ix(n: u8, gates: &[Gate], counts: Vec<Count>) -> Instruction {
    Instruction::new_with_bytes(
        qsim::id(),
        &qsim::instruction::Simulate { num_qubits: n, circuit: encode(gates), counts }.data(),
        qsim::accounts::Simulate {}.to_account_metas(None),
    )
}

/// Host-side expectation from the same library.
fn host_score(n: u32, gates: &[Gate], c: &[Count]) -> solana_qsim::Score {
    let mut amps = vec![Complex::ZERO; 1 << n];
    let mut state = State::new(&mut amps).unwrap();
    state.reset();
    for g in gates {
        state.apply(g).unwrap();
    }
    let c: Vec<_> = c.iter().map(|c| solana_qsim::Count { outcome: c.outcome, count: c.count }).collect();
    solana_qsim::score(&state, &c).unwrap()
}

fn ghz(n: u8) -> Vec<Gate> {
    let mut g = vec![Gate::H(0)];
    g.extend((1..n).map(|q| Gate::Cx(q - 1, q)));
    g
}

#[test]
fn simulate_scores_bell_states() {
    let (mut svm, payer) = setup();
    let bell = ghz(2);

    // Ideal Bell counts: fidelity 1, xeb 2^2·½ − 1 = 1.
    let ideal = counts(&[(0, 512), (3, 512)]);
    let sent = send(&mut svm, &[simulate_ix(2, &bell, ideal.clone())], &payer, &[&payer]).unwrap();
    let s = QsimScore::deserialize(&mut sent.return_data.as_slice()).unwrap();
    println!("Bell, ideal counts: {} CU, {} bytes · fidelity {} ppm, tvd {} ppm, xeb {} ppm", sent.units, sent.size, s.fidelity_ppm, s.tvd_ppm, s.xeb_ppm);
    assert!(s.fidelity_ppm >= 999_990 && s.tvd_ppm <= 10 && (s.xeb_ppm - 1_000_000).abs() <= 10);

    // A realistic noisy Bell run (made-up example numbers, like a superconducting QPU).
    let noisy = counts(&[(0, 478), (1, 21), (2, 26), (3, 499)]);
    let sent = send(&mut svm, &[simulate_ix(2, &bell, noisy.clone())], &payer, &[&payer]).unwrap();
    let s = QsimScore::deserialize(&mut sent.return_data.as_slice()).unwrap();
    let h = host_score(2, &bell, &noisy);
    println!("Bell, noisy counts: fidelity {} ppm, tvd {} ppm, xeb {} ppm", s.fidelity_ppm, s.tvd_ppm, s.xeb_ppm);
    assert_eq!((s.fidelity_ppm, s.tvd_ppm, s.xeb_ppm, s.shots), (h.fidelity_ppm, h.tvd_ppm, h.xeb_ppm, h.shots));

    // A "QPU result" that is really a uniform coin flip scores badly.
    let fake = counts(&[(0, 256), (1, 256), (2, 256), (3, 256)]);
    let sent = send(&mut svm, &[simulate_ix(2, &bell, fake)], &payer, &[&payer]).unwrap();
    let s = QsimScore::deserialize(&mut sent.return_data.as_slice()).unwrap();
    println!("Bell, uniform fake: fidelity {} ppm, tvd {} ppm, xeb {} ppm", s.fidelity_ppm, s.tvd_ppm, s.xeb_ppm);
    assert!(s.fidelity_ppm < 600_000 && s.xeb_ppm.abs() <= 10);

    // Bad input is rejected.
    assert!(send(&mut svm, &[simulate_ix(2, &bell, vec![])], &payer, &[&payer]).unwrap_err().contains("Custom(6002)"));
    assert!(send(&mut svm, &[simulate_ix(2, &[Gate::H(5)], counts(&[(0, 1)]))], &payer, &[&payer]).unwrap_err().contains("Custom(6000)"));
    assert!(send(&mut svm, &[simulate_ix(12, &bell, counts(&[(0, 1)]))], &payer, &[&payer]).unwrap_err().contains("Custom(6001)"));
}

/// CU for a GHZ circuit at every register size `simulate` supports.
#[test]
fn bench_simulate_ghz() {
    let (mut svm, payer) = setup();
    println!("\nsimulate: GHZ circuit (H + CX chain), 2 counted outcomes");
    println!("| qubits | gates | CU | CU per gate | tx bytes |");
    println!("|---|---|---|---|---|");
    for n in 2..=11u8 {
        let gates = ghz(n);
        let all_ones = (1u32 << n) - 1;
        let c = counts(&[(0, 500), (all_ones, 500)]);
        let sent = send(&mut svm, &[simulate_ix(n, &gates, c.clone())], &payer, &[&payer]).unwrap();
        let s = QsimScore::deserialize(&mut sent.return_data.as_slice()).unwrap();
        assert!(s.fidelity_ppm >= 999_990, "n={n} {:?}", s.fidelity_ppm);
        println!("| {n} | {} | {} | {} | {} |", gates.len(), sent.units, sent.units / gates.len() as u64, sent.size);
    }
}

/// The largest random circuit (layers of U on every qubit + CX ladder) per transaction.
#[test]
fn bench_simulate_random_layers() {
    let (mut svm, payer) = setup();
    println!("\nsimulate: random layers (U on every qubit, then CX between neighbours), deepest that fits in 1.4M CU and 1,232 bytes");
    println!("| qubits | layers | gates | CU |");
    println!("|---|---|---|---|");
    for n in [4u8, 6, 8, 10, 11] {
        let mut best = None;
        for layers in 1..=40 {
            let mut gates = vec![];
            for l in 0..layers {
                for q in 0..n {
                    let a = (l as u32).wrapping_mul(2654435761).wrapping_add(q as u32 * 40503);
                    gates.push(Gate::U(q, a, a.rotate_left(9), a.rotate_left(21)));
                }
                for q in (l % 2..n - 1).step_by(2) {
                    gates.push(Gate::Cx(q, q + 1));
                }
            }
            match send(&mut svm, &[simulate_ix(n, &gates, counts(&[(0, 1)]))], &payer, &[&payer]) {
                Ok(sent) if sent.size <= PACKET_DATA_SIZE => best = Some((layers, gates.len(), sent.units)),
                _ => break,
            }
        }
        if let Some((layers, gates, units)) = best {
            println!("| {n} | {layers} | {gates} | {units} |");
        }
    }
}

// ---- State accounts ----

fn new_state(svm: &mut LiteSVM, payer: &Keypair, n: u8) -> Pubkey {
    let key = Pubkey::new_unique();
    let len = qsim::state::state_len(n);
    let lamports = svm.minimum_balance_for_rent_exemption(len);
    svm.set_account(key, Account { lamports, data: vec![0; len], owner: qsim::id(), executable: false, rent_epoch: 0 }).unwrap();
    let ix = Instruction::new_with_bytes(
        qsim::id(),
        &qsim::instruction::InitState { num_qubits: n }.data(),
        qsim::accounts::InitState { authority: payer.pubkey(), state: key }.to_account_metas(None),
    );
    send(svm, &[ix], payer, &[payer]).unwrap();
    key
}

fn apply_ix(authority: Pubkey, state: Pubkey, gates: &[Gate]) -> Instruction {
    Instruction::new_with_bytes(
        qsim::id(),
        &qsim::instruction::Apply { circuit: encode(gates) }.data(),
        qsim::accounts::Apply { authority, state }.to_account_metas(None),
    )
}

fn apply_range_ix(authority: Pubkey, state: Pubkey, gate: &Gate, end: u64) -> Instruction {
    Instruction::new_with_bytes(
        qsim::id(),
        &qsim::instruction::ApplyRange { gate: encode(&[*gate]), end }.data(),
        qsim::accounts::Apply { authority, state }.to_account_metas(None),
    )
}

fn score_ix(state: Pubkey, counts: Vec<Count>) -> Instruction {
    Instruction::new_with_bytes(
        qsim::id(),
        &qsim::instruction::Score { counts }.data(),
        qsim::accounts::ScoreState { state }.to_account_metas(None),
    )
}

#[test]
fn account_state_matches_simulate() {
    let (mut svm, payer) = setup();
    let n = 6u8;
    let gates: Vec<Gate> = (0..30u32)
        .map(|i| match i % 5 {
            0 => Gate::H((i % 6) as u8),
            1 => Gate::Rz(((i + 1) % 6) as u8, i.wrapping_mul(97_000_003)),
            2 => Gate::Cx((i % 6) as u8, ((i + 2) % 6) as u8),
            3 => Gate::U(((i + 3) % 6) as u8, i * 11_000_000, i * 7_000_000, i * 3_000_000),
            _ => Gate::Ccx((i % 6) as u8, ((i + 1) % 6) as u8, ((i + 3) % 6) as u8),
        })
        .collect();
    let c = counts(&[(0, 100), (5, 40), (17, 60), (63, 9)]);

    // One transaction on the heap.
    let sent = send(&mut svm, &[simulate_ix(n, &gates, c.clone())], &payer, &[&payer]).unwrap();
    let heap = QsimScore::deserialize(&mut sent.return_data.as_slice()).unwrap();

    // The same circuit in an account: first half with `apply`, then every remaining gate split
    // into two `apply_range` calls.
    let state = new_state(&mut svm, &payer, n);
    send(&mut svm, &[apply_ix(payer.pubkey(), state, &gates[..15])], &payer, &[&payer]).unwrap();
    for g in &gates[15..] {
        let items = 1u64 << (n as usize - g.qubits().1);
        send(&mut svm, &[apply_range_ix(payer.pubkey(), state, g, items / 2)], &payer, &[&payer]).unwrap();
        send(&mut svm, &[apply_range_ix(payer.pubkey(), state, g, items)], &payer, &[&payer]).unwrap();
    }
    let sent = send(&mut svm, &[score_ix(state, c.clone())], &payer, &[&payer]).unwrap();
    let acct = QsimScore::deserialize(&mut sent.return_data.as_slice()).unwrap();

    let h = host_score(n as u32, &gates, &c);
    assert_eq!((heap.fidelity_ppm, heap.tvd_ppm, heap.xeb_ppm), (h.fidelity_ppm, h.tvd_ppm, h.xeb_ppm));
    assert_eq!((acct.fidelity_ppm, acct.tvd_ppm, acct.xeb_ppm), (h.fidelity_ppm, h.tvd_ppm, h.xeb_ppm));
    assert_eq!(heap.circuit_hash, acct.circuit_hash, "both paths hash the same gates the same way");
    let data = svm.get_account(&state).unwrap().data;
    assert_eq!(u64::from_le_bytes(data[16..24].try_into().unwrap()), 30);
}

#[test]
fn state_rules_are_enforced() {
    let (mut svm, payer) = setup();
    let state = new_state(&mut svm, &payer, 4);
    let stranger = Keypair::new();
    svm.airdrop(&stranger.pubkey(), 1_000_000_000).unwrap();

    // Only the authority can change the state; initializing twice fails.
    let err = send(&mut svm, &[apply_ix(stranger.pubkey(), state, &[Gate::H(0)])], &stranger, &[&stranger]).unwrap_err();
    assert!(err.contains("Custom(6006)"), "{err}");
    let again = Instruction::new_with_bytes(
        qsim::id(),
        &qsim::instruction::InitState { num_qubits: 4 }.data(),
        qsim::accounts::InitState { authority: payer.pubkey(), state }.to_account_metas(None),
    );
    assert!(send(&mut svm, &[again], &payer, &[&payer]).unwrap_err().contains("Custom(6004)"));

    // A half-applied gate blocks everything else until it is finished, in order.
    send(&mut svm, &[apply_range_ix(payer.pubkey(), state, &Gate::H(1), 3)], &payer, &[&payer]).unwrap();
    for ix in [apply_ix(payer.pubkey(), state, &[Gate::X(0)]), score_ix(state, counts(&[(0, 1)]))] {
        assert!(send(&mut svm, &[ix], &payer, &[&payer]).unwrap_err().contains("Custom(6007)"));
    }
    // Different gate, or going backwards: rejected.
    assert!(send(&mut svm, &[apply_range_ix(payer.pubkey(), state, &Gate::H(2), 8)], &payer, &[&payer]).unwrap_err().contains("Custom(6008)"));
    assert!(send(&mut svm, &[apply_range_ix(payer.pubkey(), state, &Gate::H(1), 2)], &payer, &[&payer]).unwrap_err().contains("Custom(6008)"));
    send(&mut svm, &[apply_range_ix(payer.pubkey(), state, &Gate::H(1), 8)], &payer, &[&payer]).unwrap();
    send(&mut svm, &[score_ix(state, counts(&[(0, 1), (2, 1)]))], &payer, &[&payer]).unwrap();

    // An account too small for the qubit count can't be initialized.
    let small = Pubkey::new_unique();
    svm.set_account(small, Account { lamports: 1_000_000_000, data: vec![0; 200], owner: qsim::id(), executable: false, rent_epoch: 0 }).unwrap();
    let ix = Instruction::new_with_bytes(
        qsim::id(),
        &qsim::instruction::InitState { num_qubits: 5 }.data(),
        vec![AccountMeta::new_readonly(payer.pubkey(), true), AccountMeta::new(small, false)],
    );
    assert!(send(&mut svm, &[ix], &payer, &[&payer]).unwrap_err().contains("Custom(6003)"));
}

/// CU per work item for each gate kind on account state, and what that means per register size.
#[test]
fn bench_account_gates() {
    let (mut svm, payer) = setup();
    let n = 16u8;
    let state = new_state(&mut svm, &payer, n);
    let chunk = 4096u64;
    println!("\napply_range on a {n}-qubit account state, {chunk} work items per transaction");
    println!("| gate | CU for {chunk} items | CU per item | items per 1.4M-CU tx |");
    println!("|---|---|---|---|");
    let mut per_item = vec![];
    for (name, gate) in [
        ("H", Gate::H(3)),
        ("H on qubit 0", Gate::H(0)),
        ("H on qubit 15", Gate::H(15)),
        ("X", Gate::X(3)),
        ("T", Gate::T(3)),
        ("RZ", Gate::Rz(3, 0x1234_5678)),
        ("U", Gate::U(3, 0x1234_5678, 0x0BAD_F00D, 0x0FED_CBA9)),
        ("U on qubit 0", Gate::U(0, 0x1234_5678, 0x0BAD_F00D, 0x0FED_CBA9)),
        ("CX", Gate::Cx(3, 9)),
        ("CX control on qubit 0", Gate::Cx(0, 9)),
        ("CZ", Gate::Cz(3, 9)),
        ("CCX", Gate::Ccx(3, 9, 12)),
    ] {
        // Measure one chunk, then finish the gate so the next one can start.
        let sent = send(&mut svm, &[apply_range_ix(payer.pubkey(), state, &gate, chunk)], &payer, &[&payer]).unwrap();
        let items = 1u64 << (n as usize - gate.qubits().1);
        let step = (1_200_000.0 * chunk as f64 / sent.units as f64) as u64;
        let mut at = chunk;
        while at < items {
            at = (at + step).min(items);
            send(&mut svm, &[apply_range_ix(payer.pubkey(), state, &gate, at)], &payer, &[&payer]).unwrap();
        }
        let cu = sent.units as f64 / chunk as f64;
        per_item.push((name, cu, gate.qubits().1));
        println!("| {name} | {} | {cu:.1} | {:.0} |", sent.units, 1_350_000.0 / cu);
    }
    println!("\nTransactions per gate by register size (work items = 2^(n−k); 1.35M usable CU per tx):");
    println!("| qubits | amplitudes | account size | H | U | CX |");
    println!("|---|---|---|---|---|---|");
    for q in [10u32, 12, 14, 16, 18, 20] {
        let txs = |name: &str| {
            let (_, cu, k) = per_item.iter().find(|p| p.0 == name).unwrap();
            ((1u64 << (q - *k as u32)) as f64 * cu / 1_350_000.0).ceil()
        };
        println!("| {q} | {} | {:.1} MiB | {} | {} | {} |", 1u64 << q, ((8u64 << q) + 128) as f64 / 1048576.0, txs("H"), txs("U"), txs("CX"));
    }
}

#[test]
fn twenty_qubit_state_works() {
    let (mut svm, payer) = setup();
    let n = 20u8;
    let state = new_state(&mut svm, &payer, n);
    // H on qubit 0 across chunks, then CX(0, 19): a 20-qubit Bell pair between the end qubits.
    for gate in [Gate::H(0), Gate::Cx(0, 19)] {
        let items = 1u64 << (n as usize - gate.qubits().1);
        // Size chunks from a measured first chunk so each stays under 1.4M CU.
        let first = 4096u64;
        let sent = send(&mut svm, &[apply_range_ix(payer.pubkey(), state, &gate, first)], &payer, &[&payer]).unwrap();
        let step = (1_200_000.0 * first as f64 / sent.units as f64) as u64;
        let mut at = first;
        let mut txs = 1;
        while at < items {
            at = (at + step).min(items);
            send(&mut svm, &[apply_range_ix(payer.pubkey(), state, &gate, at)], &payer, &[&payer]).unwrap();
            txs += 1;
        }
        println!("20 qubits: {gate:?} took {txs} transactions");
    }
    let c = counts(&[(0, 500), ((1 << 19) | 1, 500)]);
    let sent = send(&mut svm, &[score_ix(state, c)], &payer, &[&payer]).unwrap();
    let s = QsimScore::deserialize(&mut sent.return_data.as_slice()).unwrap();
    println!("20-qubit score: {} CU, fidelity {} ppm, xeb {} ppm", sent.units, s.fidelity_ppm, s.xeb_ppm);
    assert!(s.fidelity_ppm >= 999_990);
}

// ---- simulate_receipt: counts bound to a qproof receipt ----

/// Records a qproof receipt the way solana-qsim/tools/qsim.js `record` does.
fn record_receipt(svm: &mut LiteSVM, relayer: &Keypair, job: &str, circuit_hash: [u8; 32], result_hash: [u8; 32], shots: u32) -> Pubkey {
    use anchor_lang::solana_program::system_program;
    let config = Pubkey::find_program_address(&[qproof::CONFIG_SEED], &qproof::id()).0;
    if svm.get_account(&config).is_none() {
        let ix = Instruction::new_with_bytes(
            qproof::id(),
            &qproof::instruction::Initialize { relayer: relayer.pubkey() }.data(),
            qproof::accounts::Initialize { authority: relayer.pubkey(), config, system_program: system_program::ID }.to_account_metas(None),
        );
        send(svm, &[ix], relayer, &[relayer]).unwrap();
    }
    let job_id = solana_sha256_hasher::hash(job.as_bytes()).to_bytes();
    let receipt = Pubkey::find_program_address(&[qproof::RECEIPT_SEED, &job_id], &qproof::id()).0;
    let args = qproof::RecordJobArgs {
        job_id,
        source: qproof::state::JobSource::Qpu,
        backend: "braket:Cepheus-1-108Q".to_string(),
        circuit_hash,
        result_hash,
        shots,
    };
    let ix = Instruction::new_with_bytes(
        qproof::id(),
        &qproof::instruction::RecordJob { args }.data(),
        qproof::accounts::RecordJob { relayer: relayer.pubkey(), config, receipt, system_program: system_program::ID }.to_account_metas(None),
    );
    send(svm, &[ix], relayer, &[relayer]).unwrap();
    receipt
}

fn simulate_receipt_ix(receipt: Pubkey, n: u8, gates: &[Gate], counts: Vec<Count>) -> Instruction {
    Instruction::new_with_bytes(
        qsim::id(),
        &qsim::instruction::SimulateReceipt { num_qubits: n, circuit: encode(gates), counts }.data(),
        qsim::accounts::SimulateReceipt { receipt }.to_account_metas(None),
    )
}

fn chain_hash(gates: &[Gate]) -> [u8; 32] {
    let mut h = [0u8; 32];
    for g in gates {
        h = solana_sha256_hasher::hashv(&[&h, &encode(&[*g])]).to_bytes();
    }
    h
}

#[test]
fn simulate_receipt_binds_counts_to_the_receipt() {
    let (mut svm, payer) = setup();
    let gates = ghz(5);
    let real = counts(&[(0, 375), (1, 27), (3, 33), (7, 29), (15, 40), (16, 34), (24, 26), (31, 341)]);

    // The canonical text is exactly what tools/sample.py hashes.
    let expected = solana_sha256_hasher::hash(b"00000:375\n00001:27\n00011:33\n00111:29\n01111:40\n10000:34\n11000:26\n11111:341").to_bytes();
    assert_eq!(qsim::counts_hash(5, &real), expected);

    let shots = real.iter().map(|c| c.count).sum();
    let receipt = record_receipt(&mut svm, &payer, "arn:aws:braket:test-task", chain_hash(&gates), expected, shots);

    // Genuine counts: scored, and identical to plain `simulate`.
    let sent = send(&mut svm, &[simulate_receipt_ix(receipt, 5, &gates, real.clone())], &payer, &[&payer]).unwrap();
    let bound = QsimScore::deserialize(&mut sent.return_data.as_slice()).unwrap();
    let plain = send(&mut svm, &[simulate_ix(5, &gates, real.clone())], &payer, &[&payer]).unwrap();
    let plain = QsimScore::deserialize(&mut plain.return_data.as_slice()).unwrap();
    println!("simulate_receipt: {} CU, fidelity {} ppm", sent.units, bound.fidelity_ppm);
    assert_eq!((bound.fidelity_ppm, bound.xeb_ppm), (plain.fidelity_ppm, plain.xeb_ppm));

    // "Better" counts with the same shot total: rejected on-chain.
    let forged = counts(&[(0, 453), (31, 452)]);
    let err = send(&mut svm, &[simulate_receipt_ix(receipt, 5, &gates, forged)], &payer, &[&payer]).unwrap_err();
    assert!(err.contains("Custom(6011)"), "{err}");
    // The same counts claimed for a different circuit: rejected.
    let err = send(&mut svm, &[simulate_receipt_ix(receipt, 5, &ghz(4).into_iter().chain([Gate::H(4)]).collect::<Vec<_>>(), real)], &payer, &[&payer]).unwrap_err();
    assert!(err.contains("Custom(6010)"), "{err}");
}
