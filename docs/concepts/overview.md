# Miraset Chain Concepts

## Executive Summary
**Miraset Chain** is a blockchain-based decentralized GPU compute marketplace that rewards participants for running AI inference workloads. It combines blockchain settlement with off-chain GPU inference to create an economic layer for distributed AI compute resources.

## Core Concept
Miraset Chain connects **GPU providers** (workers) with **AI inference consumers** (users) through a blockchain settlement layer, enabling a decentralized marketplace for AI compute. The system rewards both **capacity** (keeping GPU resources available) and **compute** (actual inference work performed).

**Key Innovation:** A dual incentive model called **Proof of Compute & Capacity (PoCC)** that creates sustainable economics for distributed AI infrastructure.

## Participants

### 1. Workers (GPU Node Operators)
Workers are participants who contribute GPU compute resources to the network:
- Register their hardware capabilities (VRAM, GPU model, supported AI models)
- Run local LLM inference engines (Ollama or LMStudio)
- Execute inference jobs assigned by the coordinator
- Submit cryptographic proofs of work completed
- Earn rewards for both availability and compute performed

### 2. Users (AI Inference Consumers)
Users request AI inference services:
- Fund escrow accounts with tokens
- Submit inference requests (prompts, model selection, parameters)
- Receive streamed responses from assigned workers
- Pay per-token pricing for consumed compute

### 3. Coordinator (MVP Phase)
A trusted scheduler that:
- Matches jobs to capable workers based on requirements
- Monitors job execution
- Co-signs work receipts for settlement
- *(Post-MVP: becomes decentralized)*

### 4. Validators
Blockchain validators who:
- Run consensus (BFT/PoS)
- Finalize settlements every epoch (60 minutes)
- Distribute rewards based on verified work
- Secure the network

## Use Cases
1. **Cost-Efficient AI Inference**: 50-70% cost reduction for high-volume users.
2. **Decentralized AI Services**: Censorship-resistant, always-available AI inference.
3. **GPU Monetization**: Passive income for gaming rigs and data center GPUs.
4. **Privacy-Preserving AI**: Data sovereignty + verifiable execution.
5. **Research & Education**: Democratized access to AI infrastructure.
6. **Enterprise AI Deployment**: Scalable AI without capital expenditure.
