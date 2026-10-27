use {
    anchor_lang::{
        prelude::Pubkey,
        solana_program::{instruction::Instruction, system_program},
        AccountDeserialize, InstructionData, ToAccountMetas,
    },
    litesvm::LiteSVM,
    qproof::{state::JobSource, RecordJobArgs},
    solana_keypair::Keypair,
    solana_message::{Message, VersionedMessage},
    solana_signer::Signer,
    solana_transaction::versioned::VersionedTransaction,
};

fn send(svm: &mut LiteSVM, ix: Instruction, payer: &Keypair) -> bool {
    svm.expire_blockhash();
    let msg = Message::new_with_blockhash(&[ix], Some(&payer.pubkey()), &svm.latest_blockhash());
    let tx = VersionedTransaction::try_new(VersionedMessage::Legacy(msg), &[payer]).unwrap();
    svm.send_transaction(tx).is_ok()
}

fn record_ix(relayer: Pubkey, config: Pubkey, job_id: [u8; 32]) -> (Instruction, Pubkey) {
    let receipt = Pubkey::find_program_address(&[qproof::RECEIPT_SEED, &job_id], &qproof::id()).0;
    let args = RecordJobArgs {
        job_id,
        source: JobSource::CloudEmulator,
        backend: "perceval:sim".to_string(),
        circuit_hash: [2; 32],
        result_hash: [3; 32],
        shots: 1000,
    };
    let ix = Instruction::new_with_bytes(
        qproof::id(),
        &qproof::instruction::RecordJob { args }.data(),
        qproof::accounts::RecordJob {
            relayer,
            config,
            receipt,
            system_program: system_program::ID,
        }
        .to_account_metas(None),
    );
    (ix, receipt)
}

#[test]
fn test_record_job() {
    let mut svm = LiteSVM::new();
    let bytes = include_bytes!(concat!(env!("CARGO_TARGET_TMPDIR"), "/../deploy/qproof.so"));
    svm.add_program(qproof::id(), bytes).unwrap();

    let authority = Keypair::new();
    let relayer = Keypair::new();
    let stranger = Keypair::new();
    for kp in [&authority, &relayer, &stranger] {
        svm.airdrop(&kp.pubkey(), 1_000_000_000).unwrap();
    }
    let config = Pubkey::find_program_address(&[qproof::CONFIG_SEED], &qproof::id()).0;

    let ix = Instruction::new_with_bytes(
        qproof::id(),
        &qproof::instruction::Initialize { relayer: relayer.pubkey() }.data(),
        qproof::accounts::Initialize {
            authority: authority.pubkey(),
            config,
            system_program: system_program::ID,
        }
        .to_account_metas(None),
    );
    assert!(send(&mut svm, ix, &authority));

    // Only the relayer may record jobs.
    let (ix, _) = record_ix(stranger.pubkey(), config, [9; 32]);
    assert!(!send(&mut svm, ix, &stranger));

    let (ix, receipt) = record_ix(relayer.pubkey(), config, [1; 32]);
    assert!(send(&mut svm, ix, &relayer));

    let account = svm.get_account(&receipt).unwrap();
    let receipt = qproof::state::Receipt::try_deserialize(&mut account.data.as_slice()).unwrap();
    assert_eq!(receipt.job_id, [1; 32]);
    assert_eq!(receipt.source, JobSource::CloudEmulator);
    assert_eq!(receipt.backend, "perceval:sim");
    assert_eq!(receipt.shots, 1000);
    assert_eq!(receipt.index, 0);
    assert_eq!(receipt.relayer, relayer.pubkey());

    // The same job cannot be recorded twice.
    let (ix, _) = record_ix(relayer.pubkey(), config, [1; 32]);
    assert!(!send(&mut svm, ix, &relayer));
}
