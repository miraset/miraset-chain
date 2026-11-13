Нижче — **Jira-задача на BE** (Payment Service, off-chain). Frontend/checkout — поза scope цієї ітерації.

---

### Title

**[BE] AiFinPay webhook → inference credits on MSR-C + entitlement outbox**

### Component / Area

`Payment Service` (product backend, **не** `miraset-chain`)

---

### Description

**Goal**

Реалізувати backend-шлях після фінальної оплати AiFinPay: верифікація події, запис платежу, **одноразове** нарахування **inference credits** на **`client_address` (MSR-C)**, оновлення entitlements і публікація подій для control plane. Клієнт отримує **кредити** (право на LLM-ресурси мережі), **не** MIR і не transferable asset (**ADR-0017**, **ADR-0014**).

**Out of scope (цієї задачі)**

- React checkout / AiFinPay widget (окремий FE story)
- On-chain публікація grant у Master Node (policy gate — disabled)
- Subscription recurring / dunning (наступна фаза)
- Chain settlement / protocol fee split

---

### Requirements

* `POST /webhooks/aifinpay`: HMAC SHA-256 по raw body, replay window, dedup за `event_id` (unique constraint).
* HTTP 200 лише після успішного DB commit; transient failure — retryable, без `processed`.
* Payment Verifier: final status, `client_order_id`, сума, валюта, merchant/account; збіг з `orders`.
* Order **обов’язково** містить валідний `client_address` (`msrc1…`, MSR-C class `0x01`).
* Один atomic TX: `payments` + append `credit_ledger` + `entitlement_grants` + `domain_outbox`.
* Ledger keyed by **`client_address`**, не по `user_id`.
* `user_id` — лише мапінг web-акаунта → MSR-C (API/provision helper у scope, якщо ще немає).
* Outbox: `credits.balance_changed`, `access.entitlements_updated` з `client_address`, `grant_id`; consumers ідемпотентні.
* Capability gate: обробка лише rails зі статусом `verified` (документ + env flag).
* Secrets: `AIFINPAY_SECRET`, `AIFINPAY_WEBHOOK_SECRET` — тільки Payment Service.
* Метрики + audit log на grant/reversal paths.

---

### Technical Notes

| Тема | Рішення |
|------|---------|
| **Де живе код** | Off-chain **Payment Service** (modular monolith ок: модулі Order, Verifier/Webhook, Credits). |
| **MSR-C** | Bech32m HRP `msrc`, class `0x01` + Ed25519 pubkey 32B (**ADR-0017**). Відхиляти `msrw`/`msrv`/`msrg` для customer grant. |
| **Одиниця** | `credit_amount` у ledger; 1 credit = 1000 output units **лише для metering/debit**, не як «видача токена». |
| **Схема** | `orders`, `payments`, `credit_ledger`, `webhook_events`, `entitlement_grants`, `domain_outbox`, `audit_log` (реф. `PAYMENTS_AIFINPAY.md` §3, §5.1). |
| **Order kinds** | v1: `credits_topup`; `subscription` — stub hooks, без recurring charge. |
| **AML** | Crypto hold → fail-closed `pending_review`; без grant до resolve (заглушка policy OK з TODO). |

---

### Коментарі для розробників (BE)

**1. Чому не on-chain**  
Платіж і grant credits — **джерело істини Postgres**. `miraset-chain` не приймає webhooks і не є billing ledger. On-chain — пізніше, окремий epic.

**2. Ідемпотентність — дві осі**
- `webhook_events.event_id` — dedup доставки PSP.
- `credit_ledger.reference` (payment_id / order_id) — unique grant per payment.  
  Дубль webhook → 200, **без** другого append у ledger.

**3. Порядок commit**
```
verify HMAC → insert webhook (received) → process in worker/тій же TX:
  lock order → validate → payments → ledger → entitlements → outbox → webhook processed
→ 200
```
Ніколи не ставити `processed` до успішного ledger.

**4. Зв’язок з control plane**  
Control plane **не** читає AiFinPay. Після grant — outbox (HTTP callback / internal bus — узгодити з існуючим стеком). Мінімум: REST `GET /v1/clients/{msrc}/credits` для polling до появи bus.

**5. MSR-C без wallet**  
Web-only: при реєстрації Payment/Credits модуль створює custodial keypair → `msrc1…` і зберігає прив’язку `user_id ↔ client_address` (key custody — окремий security ticket; тут лише API + DB).

**6. Reconciliation**  
Cron/job: `payment.succeeded` без ledger row / ledger без payment → alert + DLQ replay runbook.

**7. Що не плутати**
- **Депозит на merchant** ≠ split 50/30/20 (це protocol fee від inference spend, `tokenomics.md`).
- **Credits** ≠ MIR.

---

### API / endpoints (BE, v1)

| Method | Path | Призначення |
|--------|------|-------------|
| `POST` | `/webhooks/aifinpay` | Події PSP (public, HMAC) |
| `POST` | `/v1/orders` | Створити order (internal/auth; для FE пізніше) |
| `GET` | `/v1/clients/{client_address}/credits` | Баланс по MSR-C |
| `GET` | `/v1/clients/{client_address}/entitlements` | Tier + access flags |
| `POST` | `/internal/outbox/replay` | Ops only, DLQ replay |

*Коментар:* order creation може бути internal-only, якщо FE ще немає; для інтег-тестів достатньо HTTP + sandbox webhook fixture.

---

### Acceptance Criteria

* [ ] Sandbox `payment.succeeded` → один `credit_ledger` append, баланс на вірному `msrc1…`.
* [ ] Повторний `event_id` → 200, баланс не змінюється.
* [ ] Invalid HMAC / amount mismatch / invalid MSR-C → no ledger change, 4xx.
* [ ] Pending/failed events → no grant.
* [ ] Outbox містить `credits.balance_changed` + `access.entitlements_updated`; повторна обробка outbox не дублює side effects (idempotency key).
* [ ] Integration tests: webhook fixture, concurrent duplicate delivery, DB transaction rollback on failure.
* [ ] Міграції reversible; індекс `(client_address, created_at)` на ledger.
* [ ] Metrics: `payment_to_grant_latency_seconds`, `webhook_duplicates_total`, `grant_failures_total`.
* [ ] Runbook: DLQ / replay / reconciliation mismatch (markdown у репо).

---

### Definition of Done

* [ ] BE merged у Payment Service repo/module
* [ ] Migrations applied in dev
* [ ] Integration tests green (без обов’язкового FE)
* [ ] Env/secrets документовані
* [ ] Capability gate documented (які rails увімкнені)

---

### Dependencies

* ADR-0017 (MSR-C), ADR-0014 (PSP)
* Схема БД (якщо ще немає — перший під-таск «payment schema migration»)
* Sandbox keys + capability record (мінімум один verified rail)

### Suggested subtasks (optional)

1. **[BE] Payment DB migrations** (orders, ledger, webhooks, outbox)
2. **[BE] Order Service + MSR-C validation**
3. **[BE] Webhook handler + Payment Verifier**
4. **[BE] Credits grant + entitlement + outbox**
5. **[BE] Reconciliation job + metrics**

---


### NOTE

Підключити на мерчанті AiFinPay адреси тестові EVM,Solana,Aptos,Near (всі доступні мережі)