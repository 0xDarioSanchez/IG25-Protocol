#!/usr/bin/env ts-node
// @ts-nocheck
/**
 * Protocol Contract Integration Test
 * Tests: init, register judges, create dispute, vote, reveal, check winner
 */

import { createPublicClient, createWalletClient, http, keccak256, toBytes, formatUnits } from "viem";
import { privateKeyToAccount } from "viem/accounts";
import { config as dotenvConfig } from "dotenv";
import * as path from "path";
import * as fs from "fs";
import { getChain } from "./utils/network";
import { getContractData } from "./utils/contract";

const envPath = path.resolve(__dirname, "../.env");
if (fs.existsSync(envPath)) {
  dotenvConfig({ path: envPath });
}

const NETWORK = process.env["NETWORK"] || "devnet";
const DEPLOYER_PRIVATE_KEY = process.env["DEPLOYER_PRIVATE_KEY_DEVNET"] || "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
const CHAIN_ID = "412346";

function generateCommitHash(vote: boolean, secret: string) {
  const voteStr = vote ? "true" : "false";
  const data = new TextEncoder().encode(voteStr + secret);
  return keccak256(data);
}

async function runProtocolTest() {
  console.log("🚀 Starting Protocol Test\n");

  const chain = getChain(NETWORK);
  if (!chain) throw new Error(`Chain ${NETWORK} not found`);

  const publicClient = createPublicClient({ chain, transport: http() });

  const deployer = privateKeyToAccount(DEPLOYER_PRIVATE_KEY as `0x${string}`);
  const judge1 = privateKeyToAccount(keccak256(toBytes(`${DEPLOYER_PRIVATE_KEY}1`)));
  const judge2 = privateKeyToAccount(keccak256(toBytes(`${DEPLOYER_PRIVATE_KEY}2`)));
  const judge3 = privateKeyToAccount(keccak256(toBytes(`${DEPLOYER_PRIVATE_KEY}3`)));
  const judge4 = privateKeyToAccount(keccak256(toBytes(`${DEPLOYER_PRIVATE_KEY}4`)));
  const judge5 = privateKeyToAccount(keccak256(toBytes(`${DEPLOYER_PRIVATE_KEY}5`)));
  const requester = privateKeyToAccount(keccak256(toBytes(`${DEPLOYER_PRIVATE_KEY}6`)));

  console.log("📋 Accounts:");
  console.log(`  Deployer: ${deployer.address}`);
  console.log(`  Requester: ${requester.address}`);

  const protocolData = getContractData(CHAIN_ID, "protocol");
  const protocolAddress = protocolData.address as `0x${string}`;
  const protocolAbi = protocolData.abi;

  console.log(`\n📄 Protocol: ${protocolAddress}\n`);

  // Fund accounts with ETH
  console.log("💸 Funding accounts with ETH...");
  const deployerWalletFund = createWalletClient({ account: deployer, chain, transport: http() });
  const accountsToFund = [judge1.address, judge2.address, judge3.address, judge4.address, judge5.address, requester.address];
  
  for (const addr of accountsToFund) {
    try {
      const hash = await deployerWalletFund.sendTransaction({
        to: addr,
        value: 100n * 10n ** 15n, // 0.1 ETH
      });
      await publicClient.waitForTransactionReceipt({ hash });
    } catch (e: any) {
      console.log(`⚠️  Failed to fund ${addr}:`, e.message.split('\n')[0]);
    }
  }
  console.log("✅ All accounts funded\n");

  // Step 1: Init
  console.log("Step 1: Initialize");
  const deployerWallet = createWalletClient({ account: deployer, chain, transport: http() });
  const dummyUsdc = "0x0000000000000000000000000000000000000001" as `0x${string}`;

  try {
    const { request } = await publicClient.simulateContract({
      account: deployer,
      address: protocolAddress,
      abi: protocolAbi,
      functionName: "init",
      args: [deployer.address, dummyUsdc],
    });
    const hash = await deployerWallet.writeContract(request);
    await publicClient.waitForTransactionReceipt({ hash });
    console.log("✅ Initialized\n");
  } catch (e: any) {
    console.log("⚠️  Already initialized or error:", e.message.split('\n')[0], "\n");
  }

  // Step 2: Register Judges
  console.log("Step 2: Register Judges");
  const judges = [
    { account: judge1, name: "Judge1" },
    { account: judge2, name: "Judge2" },
    { account: judge3, name: "Judge3" },
    { account: judge4, name: "Judge4" },
    { account: judge5, name: "Judge5" },
  ];

  for (const j of judges) {
    const wallet = createWalletClient({ account: j.account, chain, transport: http() });
    try {
      const { request } = await publicClient.simulateContract({
        account: j.account,
        address: protocolAddress,
        abi: protocolAbi,
        functionName: "registerAsJudge",
        args: [],
      });
      const hash = await wallet.writeContract(request);
      await publicClient.waitForTransactionReceipt({ hash });
      console.log(`✅ ${j.name} registered`);
    } catch (e: any) {
      console.log(`⚠️  ${j.name}:`, e.message.split('\n')[0]);
    }
  }

  // Step 3: Create Dispute
  console.log("\nStep 3: Create Dispute");
  const requesterWallet = createWalletClient({ account: requester, chain, transport: http() });
  
  try {
    const { request } = await publicClient.simulateContract({
      account: requester,
      address: protocolAddress,
      abi: protocolAbi,
      functionName: "createDisputeDirect",
      args: [1n, judge1.address, "Work not completed"],
    });
    const hash = await requesterWallet.writeContract(request);
    await publicClient.waitForTransactionReceipt({ hash });
    console.log("✅ Dispute created\n");
  } catch (e: any) {
    console.log("⚠️  Error:", e.message.split('\n')[0], "\n");
  }

  const disputeId = 1n;

  // Check dispute state
  const disputeInfo: any = await publicClient.readContract({
    address: protocolAddress,
    abi: protocolAbi,
    functionName: "getDispute",
    args: [disputeId],
  });
  console.log(`\nDispute ${disputeId}: ID=${disputeInfo[0]}, Contract=${disputeInfo[1]}`);
  console.log(`  Requester=${disputeInfo[2]}, Beneficiary=${disputeInfo[3]}`);
  console.log(`  WaitingForJudges=${disputeInfo[4]}, IsOpen=${disputeInfo[5]}, Resolved=${disputeInfo[6]}\n`);

  // Step 4: Register to Vote
  console.log("\nStep 4: Judges Register to Vote");
  for (const j of judges) {
    // First check if they're registered
    const judgeInfo: any = await publicClient.readContract({
      address: protocolAddress,
      abi: protocolAbi,
      functionName: "getJudge",
      args: [j.account.address],
    });
    console.log(`${j.name} info: Address=${judgeInfo[0]}, Balance=${judgeInfo[1]}, Rep=${judgeInfo[2]}`);
    
    const wallet = createWalletClient({ account: j.account, chain, transport: http() });
    try {
      const { request } = await publicClient.simulateContract({
        account: j.account,
        address: protocolAddress,
        abi: protocolAbi,
        functionName: "registerToVote",
        args: [disputeId],
      });
      const hash = await wallet.writeContract(request);
      await publicClient.waitForTransactionReceipt({ hash });
      console.log(`✅ ${j.name} registered to vote`);
    } catch (e: any) {
      console.log(`⚠️  ${j.name}:`, e.message.split('\n')[0]);
    }
  }

  // Step 5: Commit Votes
  console.log("\nStep 5: Commit Votes");
  const votes = [
    { judge: judges[0], vote: true, secret: "secret1" },
    { judge: judges[1], vote: true, secret: "secret2" },
    { judge: judges[2], vote: false, secret: "secret3" },
    { judge: judges[3], vote: true, secret: "secret4" },
    { judge: judges[4], vote: false, secret: "secret5" },
  ];

  for (const v of votes) {
    const wallet = createWalletClient({ account: v.judge.account, chain, transport: http() });
    const commitHash = generateCommitHash(v.vote, v.secret);
    console.log(`${v.judge.name}: ${v.vote ? "FOR" : "AGAINST"}`);
    
    try {
      const { request } = await publicClient.simulateContract({
        account: v.judge.account,
        address: protocolAddress,
        abi: protocolAbi,
        functionName: "commitVote",
        args: [disputeId, commitHash],
      });
      const hash = await wallet.writeContract(request);
      await publicClient.waitForTransactionReceipt({ hash });
      console.log(`✅ ${v.judge.name} committed`);
    } catch (e: any) {
      console.log(`⚠️  ${v.judge.name}:`, e.message.split('\n')[0]);
    }
  }

  // Step 6: Reveal Votes
  console.log("\nStep 6: Reveal Votes");
  for (const v of votes) {
    const wallet = createWalletClient({ account: v.judge.account, chain, transport: http() });
    const secretBytes = Array.from(new TextEncoder().encode(v.secret));
    
    try {
      const { request } = await publicClient.simulateContract({
        account: v.judge.account,
        address: protocolAddress,
        abi: protocolAbi,
        functionName: "revealVotes",
        args: [disputeId, v.vote, secretBytes],
      });
      const hash = await wallet.writeContract(request);
      await publicClient.waitForTransactionReceipt({ hash });
      console.log(`✅ ${v.judge.name} revealed`);
    } catch (e: any) {
      console.log(`⚠️  ${v.judge.name}:`, e.message.split('\n')[0]);
    }
  }

  // Step 7: Check Result
  console.log("\nStep 7: Check Results");
  const isResolved = await publicClient.readContract({
    address: protocolAddress,
    abi: protocolAbi,
    functionName: "checkIfDisputeIsResolved",
    args: [disputeId],
  });
  console.log(`Resolved: ${isResolved}`);

  if (isResolved) {
    const winner = await publicClient.readContract({
      address: protocolAddress,
      abi: protocolAbi,
      functionName: "getDisputeWinner",
      args: [disputeId],
    });
    console.log(`Winner: ${winner ? "REQUESTER" : "BENEFICIARY"}`);

    const voteResults: any = await publicClient.readContract({
      address: protocolAddress,
      abi: protocolAbi,
      functionName: "getDisputeVotes",
      args: [disputeId],
    });
    console.log(`Votes FOR: ${voteResults[0]}, AGAINST: ${voteResults[1]}`);
  }

  // Step 8: Check Rewards
  console.log("\nStep 8: Judge Rewards");
  for (const j of judges) {
    const info: any = await publicClient.readContract({
      address: protocolAddress,
      abi: protocolAbi,
      functionName: "getJudge",
      args: [j.account.address],
    });
    console.log(`${j.name}: Balance=${formatUnits(info[1], 6)} USDC, Rep=${info[2]}`);
  }

  console.log("\n✅ Test Complete!");
}

if (require.main === module) {
  runProtocolTest().catch((error) => {
    console.error("\n❌ Test failed:", error);
    process.exit(1);
  });
}

export { runProtocolTest };
