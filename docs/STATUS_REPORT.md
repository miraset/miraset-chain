# MIRASET — Статус реализации и план DEMO / Production

> Дата: 12 марта 2026  
> Объекты аудита: Wallet UI, Worker System, Blockchain Client (Node + CLI)

---

## 1. Общая сводка

| Компонент | Готовность DEMO | Готовность Prod | Статус |
|-----------|:-:|:-:|--------|
| **Blockchain Client (Node + CLI)** | 🟢 ~85 % | 🟡 ~40 % | Рабочий single-node devnet с persistence |
| **Worker System** | 🟢 ~80 % | 🟡 ~35 % | Полный цикл job → receipt → on-chain; mock fallback |
| **Wallet UI (Desktop)** | 🟢 ~75 % | 🟡 ~30 % | Tauri + Next.js, базовые операции работают |
| **Indexer** | 🔴 ~0 % | 🔴 ~0 % | Пустой placeholder |

---

## 2. Blockchain Client (Node + CLI)

### 2.1 Что уже сделано ✅

| Функция | Файлы | Деталь |
|---------|-------|--------|
| **Transaction model** (14 типов) | `miraset-core/src/types.rs` | Transfer, ChatSend, CreateObject, MutateObject, TransferObject, RegisterWorker, SubmitResourceSnapshot, CreateJob, AssignJob, SubmitJobResult, AnchorReceipt, ChallengeJob, MoveCall, PublishModule |
| **Object-centric state** (Sui-like) | `miraset-node/src/state.rs` | Objects, version control, ownership index, ObjectData (Account, WorkerRegistration, ResourceSnapshot, InferenceJob, JobResult, EpochBatch, ReceiptAnchor) |
| **RPC server** (Axum) | `miraset-node/src/rpc.rs` | 9 эндпоинтов: `/health`, `/status`, `/ping`, `/balance/{addr}`, `/nonce/{addr}`, `/block/latest`, `/block/{height}`, `/events`, `/chat/messages`, `/tx/submit` |
| **Persistence** (Sled) | `miraset-node/src/storage.rs` | Блоки (bincode), events (JSON), balances, nonces — 209 строк с тестами |
| **Block producer** | `miraset-node/src/lib.rs` | Периодичный `run_block_producer` с настраиваемым интервалом |
| **Epoch management** | `miraset-node/src/epoch.rs` | 60-мин эпохи, Submit/Challenge windows, capacity + compute rewards (351 строк с unit-тестами) |
| **Gas metering** | `miraset-node/src/gas.rs` | GasConfig, GasBudget, GasStatus, per-operation costs (382 строки) |
| **PoCC consensus** | `miraset-node/src/pocc.rs` | ValidatorSet, validator requirements (16 GiB VRAM, 3 models, stake), consensus weight, heartbeats (601 строка) |
| **Move VM** (scaffold) | `miraset-node/src/move_vm.rs` | Placeholder: publish/execute modules, sessions, typed values (387 строк) |
| **Transaction executor** | `miraset-node/src/executor.rs` | Gas-metered Transfer, CreateObject, MutateObject, TransferObject, MoveCall, PublishModule (370 строк) |
| **CLI** (`miraset`) | `miraset-cli/src/main.rs` | `node start`, `wallet new/list/balance/transfer/export/import`, `chat send/list` |
| **Config system** | `miraset-cli/src/main.rs` | CLI > env > file (`miraset.toml`) > defaults; `MIRASET_*` env vars |
| **Crypto (ed25519)** | `miraset-core/src/crypto.rs` | KeyPair generate/from_bytes, sign/verify, Address |
| **Signature verification** | `miraset-node/src/state.rs` | Zero-sig-then-hash pattern for Transfer & ChatSend |
| **Docker** | `docker-compose.yml` | Multi-node devnet (4 инстанса, порты 9944–9947) |
| **Unit тесты** | `state.rs`, `epoch.rs`, `storage.rs`, `pocc.rs` | ~30+ unit-тестов покрывают основные сценарии |
| **Integration тесты** | `tests/integration_tests.rs` | 7 тестов: balance, nonce, latest block, block by height, events, chat, error cases |

### 2.2 Что НЕ сделано / ограничения ⚠️

| Проблема | Влияние | Приоритет |
|----------|---------|-----------|
| **Нет P2P networking** | Single-node only, нет репликации | 🔴 Prod |
| **Нет real consensus** | Block producer — простой таймер, нет голосования валидаторов | 🔴 Prod |
| **Move VM — placeholder** | `MoveCall`/`PublishModule` — заглушки, возвращают mock результаты | 🟡 |
| **Signature verify** только для Transfer/ChatSend | Остальные 12 типов TX не верифицируются | 🔴 Demo+Prod |
| **Нет mempool prioritization** | Все TX включаются FIFO | 🟡 Prod |
| **state_root = [0; 32]** | Нет Merkle root, нет state proofs | 🔴 Prod |
| **Epoch settlement не привязан к block production** | `update_epoch()` вызывается вручную | 🟡 Demo |
| **Нет rate limiting / auth** на RPC | DoS-уязвимость | 🔴 Prod |
| **Нет TLS** на RPC | Трафик нешифрован | 🔴 Prod |
| **In-memory state + Sled** | Нет горизонтального масштабирования | 🟡 Prod |
| **Нет pruning / compaction** | Sled будет расти бесконтрольно | 🟡 Prod |
| **Нет CORS** на RPC | Проблемы при подключении из браузера | 🟢 Demo |

---

## 3. Worker System

### 3.1 Что уже сделано ✅

| Функция | Файлы | Деталь |
|---------|-------|--------|
| **Worker HTTP server** | `miraset-worker/src/lib.rs` | Axum router: `/health`, `/status`, `/ping`, `/jobs/accept`, `/jobs/run`, `/jobs/{id}/stream`, `/jobs/{id}/report`, `/jobs/{id}/status` (498 строк) |
| **Job lifecycle** | `lib.rs` | Полный цикл: Accept → Running → Completed → Receipt → On-chain submit |
| **Inference backend** (Ollama) | `backend.rs` | Реальный Ollama `/api/generate` + mock fallback (308 строк) |
| **Mock backend** | `backend.rs` | `MockBackend` для тестов без GPU |
| **Receipt system** | `receipt.rs` | `ReceiptPayload` с canonical bincode + blake3 hashing, signing, verification (350 строк, 4 теста) |
| **Node client** | `node_client.rs` | `register_worker`, `submit_job_result`, `anchor_receipt` — полные signed TX (183 строки) |
| **On-chain registration** | `main.rs` | При старте пытается зарегистрироваться; failure не блокирует |
| **Coordinator co-signature** | `receipt.rs` | Поле `coordinator_signature` готово (но coordinator не реализован) |
| **Backend trait** | `backend.rs` | `InferenceBackend` trait → легко добавить vLLM / TensorRT-LLM |

### 3.2 Что НЕ сделано / ограничения ⚠️

| Проблема | Влияние | Приоритет |
|----------|---------|-----------|
| **Нет job scheduler / coordinator** | Нет автоматического назначения jobs воркерам | 🔴 Demo |
| **Нет heartbeat reporting** | Worker не отправляет периодические heartbeats на ноду | 🟡 Demo |
| **Нет VRAM мониторинга** | `ResourceSnapshot` TX создаётся вручную | 🟡 Demo |
| **Нет streaming SSE** | `/jobs/{id}/stream` возвращает snapshot, не SSE | 🟡 Demo |
| **Tokenizer = split_whitespace** | Неточный подсчёт токенов | 🟡 Prod |
| **price_per_token = 10 (hardcoded)** | Не читается из epoch config | 🟡 Demo |
| **Нет retry / queue** | Если Ollama отвечает медленно — timeout, нет retry | 🟡 Prod |
| **Нет GPU health check** | Worker не проверяет доступность GPU | 🟡 Prod |
| **Нет authentication** | Любой может отправлять job requests | 🔴 Prod |
| **Нет multi-model concurrency** | Один job за раз (sequential) | 🟡 Prod |
| **Mock fallback без флага** | Автоматически падает в mock — может скрыть реальные ошибки | 🟡 Demo |
| **vLLM / TensorRT-LLM** | Только Ollama backend реализован | 🟡 Prod |
| **Coordinator node** | Не реализован (для co-signing receipts) | 🔴 Prod |
| **Challenge/dispute** | На chain есть ChallengeJob TX, но dispute resolution не реализована | 🔴 Prod |

---

## 4. Wallet UI (Desktop)

### 4.1 Что уже сделано ✅

| Функция | Файлы | Деталь |
|---------|-------|--------|
| **Tauri app** | `wallet/src-tauri/src/main.rs` | 9 Tauri commands: `get_config`, `set_rpc_url`, `list_accounts`, `create_account`, `import_account`, `export_secret`, `get_balance`, `transfer`, `check_connections` (258 строк) |
| **Wallet crate** | `miraset-wallet/src/lib.rs` | JSON-based key store, create/import/export/list (335 строк) |
| **Next.js UI** | `wallet/src/app/page.tsx` | Single-page: account list, create/import, balance display, transfer SECCO, RPC config (757 строк) |
| **Connection monitoring** | `page.tsx` | Real-time polling каждые 10 сек: RPC / Worker / Ollama status с цветными индикаторами |
| **Transfer signing** | `main.rs` (Tauri) | Полная цепочка: get_nonce → build TX → sign → submit |
| **Responsive UI** | `page.tsx` | Dark theme, Tailwind CSS, responsive grid |
| **Config persistence** | `main.rs` (Tauri) | JSON config в `AppData/miraset-wallet/config.json` |
| **Error handling** | `page.tsx` | Unified Status bar (idle/loading/error/success) |
| **Build scripts** | `tools/launcher/` | `build-and-package.bat` / `.sh` для full desktop bundle |

### 4.2 Что НЕ сделано / ограничения ⚠️

| Проблема | Влияние | Приоритет |
|----------|---------|-----------|
| **Нет transaction history** | Пользователь не видит историю отправок/получений | 🔴 Demo |
| **Нет block explorer** | Нельзя посмотреть блоки/транзакции из UI | 🟡 Demo |
| **Нет QR-code** для адреса | Неудобно на мобильном | 🟡 Demo |
| **Нет multi-account balances** | Balance запрашивается последовательно при загрузке | 🟡 Demo |
| **Wallet file = plaintext JSON** | Приватные ключи хранятся в `wallet.json` без шифрования! | 🔴 Demo+Prod |
| **Нет password/PIN protection** | Кто угодно с доступом к файлу может украсть ключи | 🔴 Prod |
| **Нет backup/recovery** | Нет mnemonic/seed phrase, только hex import/export | 🔴 Prod |
| **Нет notification** о входящих переводах | Баланс обновляется только вручную | 🟡 Demo |
| **Single-page app** | Всё в 1 файле (757 строк) — тяжело поддерживать | 🟡 Prod |
| **Нет auto-refresh balance** | Только по кнопке Refresh | 🟡 Demo |
| **Нет worker management** в UI | Нельзя зарегистрировать/мониторить workers из кошелька | 🟡 Demo |
| **Нет job submission** из UI | Нельзя отправить inference request | 🟡 Demo |
| **Нет settings page** | Все настройки на главном экране | 🟡 Demo |
| **Нет i18n** | Только английский | 🟢 Prod |
| **Tauri v1** (не v2) | `@tauri-apps/api/tauri` — устаревший import path | 🟡 Prod |
| **Нет auto-updater** | Нет механизма обновления приложения | 🟡 Prod |

---

## 5. Общие слабые места (cross-cutting)

| Проблема | Компоненты | Приоритет |
|----------|-----------|-----------|
| **Indexer = пустой placeholder** | Нет исторических запросов, нет поиска | 🔴 Demo |
| **Нет WebSocket / subscription** | Wallet не получает обновления в реальном времени | 🟡 Demo |
| **Нет CI/CD pipeline** | Нет автоматических билдов и тестов | 🟡 Prod |
| **Нет benchmarks** | Неизвестна пропускная способность | 🟡 Prod |
| **Signature scheme inconsistency** | Transfer/Chat используют zero-sig pattern; Worker registration и другие TX подписывают кастомный payload | 🔴 Prod |

---

## 6. План: DEMO версия

> Цель: запустить локальный devnet и показать end-to-end flow (создать кошелёк → получить токены → отправить job → увидеть результат).

### 6.1 Обязательно для DEMO

| # | Задача | Компонент | Оценка | Детали |
|---|--------|-----------|--------|--------|
| D1 | **Job coordinator (минимальный)** | Node | 2–3 дня | Автоматический assign free jobs to registered workers. Endpoint: `POST /jobs/create` → node создаёт job + assign → push to worker |
| D2 | **Transaction history в UI** | Wallet | 1–2 дня | Подтянуть `/events?from_height=0` и показать Transfer/Chat events для выбранного аккаунта |
| D3 | **Auto-refresh balance** | Wallet | 0.5 дня | Polling balance каждые 10 сек (как connections) |
| D4 | **Epoch auto-advance** | Node | 0.5 дня | Вызывать `update_epoch()` из block producer loop |
| D5 | **Worker heartbeat loop** | Worker | 1 день | Периодический `ResourceSnapshot` TX + heartbeat |
| D6 | **Signature verify для всех TX** | Node | 1 день | Расширить verify в `submit_transaction` на все 14 типов |
| D7 | **CORS middleware** | Node | 0.5 дня | `tower-http::CorsLayer` для Axum |
| D8 | **Demo script / walkthrough** | Docs | 1 день | Shell script / markdown guide для воспроизводимого demo |
| D9 | **Encrypt wallet file** | Wallet | 1–2 дня | AES-256-GCM с password-derived key (argon2) |
| D10 | **Job submission из Wallet UI** | Wallet | 1–2 дня | Форма: model, prompt, max_tokens → CreateJob TX |

**Итого DEMO: ~10–14 рабочих дней**

### 6.2 Nice-to-have для DEMO

- Block explorer (простая таблица блоков/TX в Wallet UI)
- SSE streaming для inference output
- Worker status panel в UI
- QR-code для адреса

---

## 7. План: Production версия

> Цель: безопасный, масштабируемый, multi-node deployment.

### 7.1 Обязательно для Production

| # | Задача | Компонент | Оценка | Детали |
|---|--------|-----------|--------|--------|
| P1 | **P2P networking (libp2p)** | Node | 3–4 недели | Gossip protocol для блоков, TX relay, peer discovery |
| P2 | **Real BFT consensus** | Node | 4–6 недель | PoCC-based validator voting, block finality, fork choice |
| P3 | **State Merkle tree** | Node | 2–3 недели | Jellyfish Merkle / sparse Merkle для state root + proofs |
| P4 | **Full Move VM** | Node | 4–8 недель | Интегрировать `move-vm-runtime`, bytecode verifier, stdlib |
| P5 | **Signature verification для всех TX** | Node | 1 неделя | Единый canonical signing format |
| P6 | **Wallet encryption** | Wallet | 1 неделя | Password/PIN, encrypted keystore (BIP-39 mnemonic) |
| P7 | **Indexer (Postgres)** | Indexer | 2–3 недели | Event consumer → Postgres, GraphQL API |
| P8 | **Rate limiting / Auth** | Node + Worker | 1 неделя | Token-based auth, rate limits на RPC |
| P9 | **TLS everywhere** | All | 1 неделя | TLS для RPC, worker ↔ node, wallet ↔ node |
| P10 | **Coordinator node** | New crate | 2–3 недели | Receipt co-signing, job scheduling, dispute resolution |
| P11 | **Dispute resolution** | Node | 2 недели | Challenge → re-execution → slash/reward |
| P12 | **Tokenizer integration** | Worker | 1 неделя | tiktoken / sentencepiece для точного token count |
| P13 | **Multi-GPU / multi-model** | Worker | 2 недели | Concurrent job execution, VRAM management |
| P14 | **Monitoring (metrics)** | All | 1 неделя | Prometheus metrics, Grafana dashboards |
| P15 | **CI/CD** | Infra | 1 неделя | GitHub Actions: build, test, lint, release |
| P16 | **Database migration** | Node | 1–2 недели | Sled → RocksDB / LMDB с state pruning |
| P17 | **Wallet refactor** | Wallet | 2 недели | Component architecture, state management (zustand/jotai), pages |
| P18 | **Tauri v2 migration** | Wallet | 1 неделя | Update to Tauri 2.x API |
| P19 | **Auto-updater** | Wallet | 1 неделя | Tauri updater plugin |
| P20 | **Security audit** | All | External | Professional audit of crypto, consensus, wallet security |

**Итого Production: ~6–9 месяцев (при команде 3–5 devs)**

### 7.2 Приоритизация Production

```
Phase 1 (MVP Network):  P1, P2, P5, P6, P8, P9, P15         → 2–3 месяца
Phase 2 (Features):     P3, P7, P10, P11, P12, P14            → 2–3 месяца  
Phase 3 (Scale):        P4, P13, P16, P17, P18, P19            → 2–3 месяца
Phase 4 (Hardening):    P20 + performance tuning + load tests  → 1–2 месяца
```

---

## 8. Текущее состояние кодовой базы (метрики)

| Крейт | Строк кода | Тесты | Покрытие |
|-------|-----------|-------|----------|
| `miraset-core` | ~700 (types.rs) + crypto | ✅ Есть | Типы, сериализация |
| `miraset-node` | ~3,500+ (state 1387, pocc 601, executor 370, gas 382, epoch 351, storage 209, rpc 153, move_vm 387) | ✅ ~30+ тестов | Основные пути |
| `miraset-cli` | ~393 | ❌ Нет unit-тестов | — |
| `miraset-worker` | ~1,340 (lib 498, receipt 350, backend 308, node_client 183) | ✅ ~8 тестов | Основные пути |
| `miraset-wallet` | ~335 | ✅ Есть | CRUD операции |
| `miraset-indexer` | 17 | ❌ Placeholder | — |
| Wallet UI (Tauri) | ~258 (Rust) + 757 (TSX) | ❌ Нет | — |

---

## 9. Заключение

Проект находится в состоянии **рабочего MVP / advanced prototype**:

- **Блокчейн-клиент** — функционально богат (14 типов TX, объектная модель, epochs, gas, PoCC), но работает только как single-node devnet без реального консенсуса.
- **Worker system** — полный цикл inference-to-chain реализован, но нет координатора и автоматического назначения jobs.
- **Wallet UI** — базовые операции (create/import/transfer/balance) работают в Tauri desktop app, но нет шифрования ключей и transaction history.

**Для DEMO** нужно ~2 недели работы (координатор jobs, tx history в UI, автоматический epoch advance, шифрование кошелька).

**Для Production** нужен серьёзный infrastructure lift (P2P, consensus, Merkle proofs, security audit) — оценочно 6–9 месяцев для команды 3–5 разработчиков.

