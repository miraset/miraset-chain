# MIRASET Indexer — Назначение, архитектура и план реализации

## Зачем нужен Indexer?

### Проблема

Сейчас **вся data-access логика живёт внутри Node** (in-memory `Vec<Event>` + Sled):

```
Wallet/Explorer  →  GET /events?from_height=0&limit=100  →  Node
                                                              ↓
                                                     state.rs: linear scan
                                                     по Vec<Event> в памяти
```

Это работает для DEMO, но **ломается в production**:

| Проблема | Почему критично |
|----------|-----------------|
| **Нет фильтрации по адресу** | Wallet не может спросить «покажи историю только для адреса X» — получает ВСЕ events и фильтрует на клиенте |
| **Нет полнотекстового поиска** | Нельзя искать chat сообщения, jobs по model_id и т.д. |
| **Нет пагинации** | `limit=1000` max — при 100k+ events это неработоспособно |
| **Нагрузка на Node** | Каждый запрос к `/events` сканирует все events в памяти — O(n) |
| **Нет агрегаций** | Нельзя спросить «сколько всего токенов заработал worker X за все эпохи?» |
| **Receipt payloads не хранятся** | On-chain лежит только `receipt_hash` (32 байта). Полный receipt payload (prompt, response, timestamps) нужно где-то хранить для dispute resolution |
| **Node restart** | Events загружаются из Sled обратно в память — при 1M+ events это медленный старт |

### Решение: Indexer как отдельный сервис

```
                    ┌─────────────┐
                    │  Postgres   │
                    │  (indexed)  │
                    └──────▲──────┘
                           │ INSERT
     ┌─────────────────────┤
     │                     │
┌────▼──────┐      ┌──────┴──────┐      ┌──────────────┐
│  Indexer  │◄─────│    Node     │      │   Worker     │
│  HTTP API │ poll │  (RPC)      │      │              │
└────▲──────┘      └─────────────┘      └──────┬───────┘
     │                                         │
     │ query                          POST receipt payloads
     │                                         │
┌────┴──────┐                          ┌───────▼───────┐
│  Wallet / │                          │   Indexer     │
│  Explorer │                          │  (receipt     │
│  Dashboard│                          │   storage)    │
└───────────┘                          └───────────────┘
```

**Indexer** — это **read-optimized** сервис, который:
1. **Потребляет events** с Node RPC (polling `/events`)
2. **Записывает в Postgres** с индексами по address, type, block, epoch
3. **Хранит receipt payloads** (полные данные, которые on-chain представлены только хешем)
4. **Предоставляет rich query API** для wallets, explorers, dashboards

---

## Что конкретно должен делать Indexer

### 1. Event Indexing

Потребляет все 14 типов events из Node и раскладывает по таблицам:

```sql
-- Основная таблица events (денормализованная)
CREATE TABLE events (
    id              BIGSERIAL PRIMARY KEY,
    block_height    BIGINT NOT NULL,
    event_type      TEXT NOT NULL,        -- 'Transferred', 'JobCreated', etc.
    tx_hash         BYTEA NOT NULL,
    -- Денормализованные поля для быстрых запросов

    address_from    TEXT,                  -- sender/creator
    address_to      TEXT,                  -- recipient (if applicable)
    amount          BIGINT,               -- transfer amount (if applicable)
    object_id       TEXT,                  -- job_id, worker_id, etc.
    data            JSONB NOT NULL,        -- полный event payload
    created_at      TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX idx_events_block    ON events(block_height);
CREATE INDEX idx_events_type     ON events(event_type);
CREATE INDEX idx_events_from     ON events(address_from);
CREATE INDEX idx_events_to       ON events(address_to);
CREATE INDEX idx_events_object   ON events(object_id);
CREATE INDEX idx_events_created  ON events(created_at);

-- Агрегированная таблица для worker stats
CREATE TABLE worker_stats (
    worker_id       TEXT PRIMARY KEY,
    owner           TEXT NOT NULL,
    gpu_model       TEXT,
    vram_gib        INT,
    total_jobs       BIGINT DEFAULT 0,
    total_tokens     BIGINT DEFAULT 0,
    total_rewards    BIGINT DEFAULT 0,
    last_heartbeat   TIMESTAMPTZ,
    uptime_score     FLOAT DEFAULT 0,
    updated_at       TIMESTAMPTZ DEFAULT NOW()
);

-- История по эпохам
CREATE TABLE epoch_history (
    epoch_id         BIGINT PRIMARY KEY,
    start_time       TIMESTAMPTZ,
    end_time         TIMESTAMPTZ,
    status           TEXT,
    total_workers    INT,
    total_jobs       INT,
    total_tokens     BIGINT,
    total_rewards    BIGINT,
    data             JSONB
);

-- Receipt payloads (off-chain storage by hash)
CREATE TABLE receipt_payloads (
    receipt_hash     BYTEA PRIMARY KEY,    -- blake3 hash
    job_id           TEXT NOT NULL,
    epoch_id         BIGINT NOT NULL,
    worker_pubkey    TEXT NOT NULL,
    model_id         TEXT NOT NULL,
    prompt_hash      BYTEA NOT NULL,
    response_hash    BYTEA NOT NULL,
    output_tokens    BIGINT NOT NULL,
    price_per_token  BIGINT NOT NULL,
    timestamp_start  TIMESTAMPTZ NOT NULL,
    timestamp_end    TIMESTAMPTZ NOT NULL,
    payload          JSONB NOT NULL,       -- полный ReceiptPayload JSON
    created_at       TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX idx_receipts_job    ON receipt_payloads(job_id);
CREATE INDEX idx_receipts_worker ON receipt_payloads(worker_pubkey);
CREATE INDEX idx_receipts_epoch  ON receipt_payloads(epoch_id);
```

### 2. Query API (HTTP)

```
GET  /api/v1/events?address=<hex>&type=<type>&from=<height>&limit=<n>
GET  /api/v1/events/address/<address>?limit=<n>
GET  /api/v1/tx/<tx_hash>
GET  /api/v1/blocks?from=<height>&limit=<n>
GET  /api/v1/blocks/<height>
GET  /api/v1/workers
GET  /api/v1/workers/<worker_id>
GET  /api/v1/workers/<worker_id>/history
GET  /api/v1/jobs?status=<status>&model=<model>&limit=<n>
GET  /api/v1/jobs/<job_id>
GET  /api/v1/epochs
GET  /api/v1/epochs/<epoch_id>
GET  /api/v1/epochs/<epoch_id>/rewards
GET  /api/v1/receipts/<receipt_hash>
GET  /api/v1/stats/summary       -- total blocks, txs, workers, jobs
GET  /api/v1/stats/address/<addr> -- total sent/received/jobs
POST /api/v1/receipts            -- worker submits full receipt payload
```

### 3. Receipt Payload Storage

**Зачем**: on-chain лежит только 32-байтный `receipt_hash`. Для dispute resolution нужен полный payload (prompt, response tokens, timestamps, signatures). Indexer хранит их и отдаёт по hash.

**Flow**:
```
Worker выполняет job
  → генерирует ReceiptPayload
  → вычисляет receipt_hash = blake3(bincode(payload))
  → отправляет receipt_hash on-chain (AnchorReceipt TX)
  → отправляет полный payload в Indexer (POST /api/v1/receipts)

При dispute:
  Challenger → GET /api/v1/receipts/<hash> → получает полный payload
  → верифицирует hash locally
  → может re-execute и сравнить результаты
```

### 4. Uptime Sampling (опционально)

Indexer может периодически пинговать зарегистрированные workers:
- `GET worker_endpoint/health`
- Записывать результаты в `worker_stats.uptime_score`
- Предоставлять evidence для slashing при длительном downtime

---

## Кто использует Indexer

| Потребитель | Что запрашивает | Сейчас (без Indexer) |
|-------------|----------------|---------------------|
| **Wallet UI** | История TX по адресу | Тянет ВСЕ events, фильтрует на клиенте |
| **Block Explorer** | Поиск блоков, TX, адресов | Не существует |
| **Dashboard** | Статистика: workers, jobs, rewards | Нет агрегаций |
| **Worker** | Сабмит receipt payload | Некуда отправить, payload теряется |
| **Dispute system** | Получить полный receipt для проверки | Невозможно — payload нигде не хранится |
| **Analytics** | Сколько токенов обработано за неделю? | Нет SQL для агрегаций |

---

## Архитектура Indexer сервиса

```
miraset-indexer/
├── src/
│   ├── lib.rs           -- публичный API крейта
│   ├── main.rs          -- бинарник (HTTP server)
│   ├── poller.rs        -- polling events из Node RPC
│   ├── db.rs            -- Postgres connection + migrations
│   ├── models.rs        -- Rust struct ↔ SQL types
│   ├── handlers.rs      -- Axum HTTP handlers
│   └── receipt_store.rs -- receipt payload storage
├── migrations/
│   └── 001_init.sql     -- CREATE TABLE statements
└── Cargo.toml
```

### Конфиг

```toml
[indexer]
node_rpc_url = "http://127.0.0.1:9944"
database_url = "postgres://miraset:miraset@localhost/miraset_indexer"
listen_addr = "127.0.0.1:9955"
poll_interval_ms = 2000
```

---

## Сравнение с аналогами

| Проект | Аналог Indexer |
|--------|---------------|
| **Ethereum** | The Graph, Etherscan backend |
| **Sui** | Sui Indexer (Postgres + GraphQL) |
| **Solana** | Helius, Triton, Geyser plugins |
| **Cosmos** | Mintscan backend |

Все серьёзные блокчейны имеют indexer — это **стандартный компонент** экосистемы.

---

## Приоритет реализации

| Фаза | Что | Оценка |
|------|-----|--------|
| **Phase 1** (DEMO+) | Polling events → Postgres, базовый query API (events by address) | 1 неделя |
| **Phase 2** (Prod) | Receipt payload storage, worker stats aggregation | 1 неделя |
| **Phase 3** (Prod) | Full query API, pagination, GraphQL, dashboard support | 1–2 недели |
| **Phase 4** (Prod) | Uptime sampling, real-time WebSocket subscriptions | 1 неделя |

