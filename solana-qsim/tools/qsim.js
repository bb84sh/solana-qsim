// solana-qsim client: score measured counts against a circuit inside the qsim program.
// Run from the repository root:
//
//   node solana-qsim/tools/qsim.js simulate <circuit.json> <counts.json> [--dry-run] [--skip-client-checks]
//       scores the counts on-chain and prints the score. If a qproof receipt exists for the
//       counts' job, it uses `simulate_receipt`: the program itself checks that the circuit and
//       the counts are the ones the receipt records. --dry-run only simulates the transaction
//       (free); --skip-client-checks sends even counts this client would reject, to show the
//       program rejecting them.
//   node solana-qsim/tools/qsim.js init-qproof
//       creates qproof's config once per cluster, with the fee payer as the relayer
//   node solana-qsim/tools/qsim.js record <counts.json>
//       records the run in qproof (receipt: job id, source, backend, circuit hash, counts hash)
//   node solana-qsim/tools/qsim.js show <signature>
//       decodes the score returned by an earlier `simulate` transaction
//
// Before scoring or recording, the counts file's result_hash is recomputed (a hand-edited file
// fails), and if a qproof receipt exists for its job, the receipt's hashes must match the files.
//
// circuit.json: from tools/qasm2qsim.py. counts.json: from tools/sample.py (or a QPU run).
// CLUSTER=devnet (default); BB84_KEYPAIR is the fee payer (default keys/devnet-wallet.json if
// present, else ~/.config/solana/id.json).
import {
  ComputeBudgetProgram, Connection, Keypair, PublicKey, Transaction, TransactionInstruction,
  clusterApiUrl, sendAndConfirmTransaction,
} from "@solana/web3.js";
import { createHash } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import { homedir } from "node:os";

const CLUSTER = process.env.CLUSTER || "devnet";
// Alchemy URLs from .env when loaded (set -a; . ./.env), else the public endpoint.
const RPC_URL = (CLUSTER === "devnet" ? process.env.SOLANA_RPC_URL_DEVNET : CLUSTER === "mainnet-beta" ? process.env.SOLANA_RPC_URL : null) || clusterApiUrl(CLUSTER);
const IDL = JSON.parse(readFileSync("target/idl/qsim.json", "utf8"));
const PROGRAM_ID = new PublicKey(IDL.address);
const PACKET_DATA_SIZE = 1232;
const QPROOF_IDL = JSON.parse(readFileSync("target/idl/qproof.json", "utf8"));
const QPROOF_ID = new PublicKey(QPROOF_IDL.address);
const SOURCES = ["LocalSimulator", "CloudEmulator", "Qpu"];
const SYSTEM_PROGRAM = new PublicKey("11111111111111111111111111111111");

const connection = new Connection(RPC_URL, "confirmed");
const KEYPAIR = process.env.BB84_KEYPAIR || (existsSync("keys/devnet-wallet.json") ? "keys/devnet-wallet.json" : `${homedir()}/.config/solana/id.json`);
const payer = Keypair.fromSecretKey(Uint8Array.from(JSON.parse(readFileSync(KEYPAIR, "utf8"))));
const explorer = (sig) => `https://explorer.solana.com/tx/${sig}${CLUSTER === "mainnet-beta" ? "" : `?cluster=${CLUSTER}`}`;

const u32 = (n) => { const b = Buffer.alloc(4); b.writeUInt32LE(n); return b; };
const discriminator = (name) => Buffer.from(IDL.instructions.find((i) => i.name === name).discriminator);

const sha256 = (data) => createHash("sha256").update(data).digest();
const configPda = () => PublicKey.findProgramAddressSync([Buffer.from("config")], QPROOF_ID)[0];
// Receipts are keyed by sha256(job id). Published counts files may carry only that hash
// (job_id_sha256) when the job id itself identifies an account (e.g. a Braket task ARN).
const jobIdHash = (counts) => (counts.job_id ? sha256(counts.job_id) : counts.job_id_sha256 ? Buffer.from(counts.job_id_sha256, "hex") : null);
const receiptPda = (jobHash) => PublicKey.findProgramAddressSync([Buffer.from("receipt"), jobHash], QPROOF_ID)[0];

/** Same canonical form as tools/sample.py: sorted "bits:count" lines. */
const countsHash = (counts) => sha256(Object.entries(counts).sort(([a], [b]) => (a < b ? -1 : 1)).map(([b, c]) => `${b}:${c}`).join("\n")).toString("hex");

function checkCountsFile(counts) {
  if (counts.result_hash && countsHash(counts.counts) !== counts.result_hash) {
    throw new Error("counts were changed after measurement: result_hash does not match the counts");
  }
}

/** The qproof receipt for this run, if one exists: { address, source, backend, circuitHash, resultHash, shots, slot }. */
async function fetchReceipt(jobHash) {
  const address = receiptPda(jobHash);
  const info = await connection.getAccountInfo(address);
  if (!info) return null;
  const d = info.data;
  let o = 8 + 32;
  const source = SOURCES[d[o]]; o += 1;
  const len = d.readUInt32LE(o); o += 4;
  const backend = d.subarray(o, o + len).toString("utf8"); o += len;
  const circuitHash = d.subarray(o, o + 32).toString("hex"); o += 32;
  const resultHash = d.subarray(o, o + 32).toString("hex"); o += 32;
  const shots = d.readUInt32LE(o); o += 4 + 32 + 8;
  const slot = Number(d.readBigUInt64LE(o));
  return { address: address.toBase58(), source, backend, circuitHash, resultHash, shots, slot };
}

/** Qiskit-style bitstrings ("1010", leftmost = highest qubit) -> sorted [outcome, count]. */
function toCounts(counts, n) {
  return Object.entries(counts)
    .map(([bits, c]) => {
      if (bits.length !== n || !/^[01]+$/.test(bits)) throw new Error(`bad bitstring ${bits} for ${n} qubits`);
      return [parseInt(bits, 2), c];
    })
    .sort((a, b) => a[0] - b[0]);
}

function simulateIx(circuit, counts, receipt) {
  const code = Buffer.from(circuit.hex, "hex");
  const pairs = toCounts(counts.counts, circuit.num_qubits);
  const data = Buffer.concat([
    discriminator(receipt ? "simulate_receipt" : "simulate"),
    Buffer.from([circuit.num_qubits]),
    u32(code.length), code,
    u32(pairs.length), ...pairs.map(([o, c]) => Buffer.concat([u32(o), u32(c)])),
  ]);
  const keys = receipt ? [{ pubkey: new PublicKey(receipt), isSigner: false, isWritable: false }] : [];
  return new TransactionInstruction({ programId: PROGRAM_ID, keys, data });
}

/** Borsh QsimScore: u8 num_qubits, u64 shots, u32 outcomes, u32 fidelity, u32 tvd, i64 xeb, [u8; 32] hash. */
function decodeScore(b64) {
  const b = Buffer.from(b64, "base64");
  return {
    numQubits: b[0],
    shots: Number(b.readBigUInt64LE(1)),
    outcomes: b.readUInt32LE(9),
    fidelity: b.readUInt32LE(13) / 1e6,
    tvd: b.readUInt32LE(17) / 1e6,
    xeb: Number(b.readBigInt64LE(21)) / 1e6,
    circuitHash: b.subarray(29, 61).toString("hex"),
  };
}

function printScore(s, units) {
  console.log(`  qubits ${s.numQubits}, shots ${s.shots}, distinct outcomes ${s.outcomes}`);
  console.log(`  fidelity ${s.fidelity.toFixed(6)}   (1 = matches the ideal distribution)`);
  console.log(`  tvd      ${s.tvd.toFixed(6)}   (0 = identical)`);
  console.log(`  xeb      ${s.xeb.toFixed(6)}   (about 0 for random noise)`);
  console.log(`  circuit hash ${s.circuitHash}${units != null ? `\n  ${units.toLocaleString("en-US")} compute units` : ""}`);
}

const commands = {
  async simulate(circuitPath, countsPath, ...flags) {
    const circuit = JSON.parse(readFileSync(circuitPath, "utf8"));
    const counts = JSON.parse(readFileSync(countsPath, "utf8"));
    const clientChecks = !flags.includes("--skip-client-checks");
    if (clientChecks && counts.circuit_hash !== circuit.circuit_hash) throw new Error("counts were measured for a different circuit");
    if (clientChecks) checkCountsFile(counts);
    const receipt = jobIdHash(counts) ? await fetchReceipt(jobIdHash(counts)) : null;
    if (receipt) {
      if (clientChecks && (receipt.resultHash !== counts.result_hash || receipt.circuitHash !== circuit.circuit_hash)) {
        throw new Error(`qproof receipt ${receipt.address} records different hashes: these are not the counts the relayer recorded`);
      }
      console.log(`qproof receipt ${receipt.address}: ${receipt.source} on ${receipt.backend}, slot ${receipt.slot} → simulate_receipt (checked on-chain)`);
    }
    const tx = new Transaction().add(ComputeBudgetProgram.setComputeUnitLimit({ units: 1_400_000 }), simulateIx(circuit, counts, receipt?.address));
    tx.feePayer = payer.publicKey;
    tx.recentBlockhash = (await connection.getLatestBlockhash()).blockhash;
    tx.sign(payer);
    const size = tx.serialize().length;
    if (size > PACKET_DATA_SIZE) throw new Error(`transaction is ${size} bytes (limit ${PACKET_DATA_SIZE}): too many gates or distinct outcomes for one transaction`);
    console.log(`${circuit.name}: ${circuit.num_qubits} qubits, ${circuit.gates} gates · counts from ${counts.backend} (${counts.source}) · ${size}-byte transaction`);

    let score, units;
    if (flags.includes("--dry-run")) {
      const sim = await connection.simulateTransaction(tx);
      if (sim.value.err) throw Object.assign(new Error(JSON.stringify(sim.value.err)), { logs: sim.value.logs });
      score = decodeScore(sim.value.returnData.data[0]);
      units = sim.value.unitsConsumed;
      console.log("dry run (simulated, nothing sent):");
    } else {
      const sig = await sendAndConfirmTransaction(connection, tx, [payer]);
      const got = await connection.getTransaction(sig, { maxSupportedTransactionVersion: 0, commitment: "confirmed" });
      score = decodeScore(got.meta.returnData.data[0]);
      units = got.meta.computeUnitsConsumed;
      console.log(`scored on ${CLUSTER}: ${explorer(sig)}`);
    }
    printScore(score, units);
    if (score.circuitHash !== circuit.circuit_hash) throw new Error("on-chain circuit hash differs from the circuit file");
  },

  async "init-qproof"() {
    if (await connection.getAccountInfo(configPda())) return console.log(`qproof config exists: ${configPda().toBase58()}`);
    const def = QPROOF_IDL.instructions.find((i) => i.name === "initialize");
    const keys = [
      { pubkey: payer.publicKey, isSigner: true, isWritable: true }, { pubkey: configPda(), isSigner: false, isWritable: true },
      { pubkey: SYSTEM_PROGRAM, isSigner: false, isWritable: false },
    ];
    const data = Buffer.concat([Buffer.from(def.discriminator), payer.publicKey.toBuffer()]);
    const sig = await sendAndConfirmTransaction(connection, new Transaction().add(new TransactionInstruction({ programId: QPROOF_ID, keys, data })), [payer]);
    console.log(`qproof config ${configPda().toBase58()} (relayer ${payer.publicKey.toBase58()}): ${explorer(sig)}`);
  },

  async record(countsPath) {
    const counts = JSON.parse(readFileSync(countsPath, "utf8"));
    const jobHash = jobIdHash(counts);
    if (counts.format !== "bb84-qsim-counts-v1" || !jobHash || !counts.result_hash) throw new Error("not a bb84-qsim-counts-v1 file with a job id (or its sha256) and result_hash");
    checkCountsFile(counts);
    const existing = await fetchReceipt(jobHash);
    if (existing) return console.log(`receipt exists: ${existing.address}`);
    const backend = Buffer.from(counts.backend, "utf8");
    const def = QPROOF_IDL.instructions.find((i) => i.name === "record_job");
    const data = Buffer.concat([
      Buffer.from(def.discriminator), jobHash, Buffer.from([SOURCES.indexOf(counts.source)]),
      u32(backend.length), backend, Buffer.from(counts.circuit_hash, "hex"), Buffer.from(counts.result_hash, "hex"), u32(counts.shots),
    ]);
    const receipt = receiptPda(jobHash);
    const keys = [
      { pubkey: payer.publicKey, isSigner: true, isWritable: true }, { pubkey: configPda(), isSigner: false, isWritable: true },
      { pubkey: receipt, isSigner: false, isWritable: true }, { pubkey: SYSTEM_PROGRAM, isSigner: false, isWritable: false },
    ];
    const sig = await sendAndConfirmTransaction(connection, new Transaction().add(new TransactionInstruction({ programId: QPROOF_ID, keys, data })), [payer]);
    console.log(`qproof receipt ${receipt.toBase58()} (${counts.source}, ${counts.backend}): ${explorer(sig)}`);
  },

  async show(sig) {
    const got = await connection.getTransaction(sig, { maxSupportedTransactionVersion: 0, commitment: "confirmed" });
    if (!got?.meta?.returnData) throw new Error("no qsim score in that transaction");
    printScore(decodeScore(got.meta.returnData.data[0]), got.meta.computeUnitsConsumed);
  },
};

const [cmd, ...args] = process.argv.slice(2);
if (!commands[cmd]) {
  console.error("usage: node solana-qsim/tools/qsim.js <init-qproof | record counts.json | simulate circuit.json counts.json [--dry-run] | show signature>");
  process.exit(1);
}
try {
  await commands[cmd](...args);
} catch (err) {
  console.error(err.message);
  if (err.logs) console.error(err.logs.join("\n"));
  process.exit(1);
}
