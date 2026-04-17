Вводные данные и зaпрос по фин. части SECCO
Вводные

Блокчейн кратко

Драфтовые соглашенные награды за блок

*к-во блокок как в SUI
Sui block time is 0.09s
https://chainspect.app/chain/sui

Capacity Rewards - за блок 1 Монета
Compute Rewards - за блок от нуля максимум до 10 Монет пропопрционально между всеми участинками от доли вклада в этом блоке.

Proof of Compute & Capacity (PoCC)
The consensus mechanism that powers Miraset Chain's economic model.

Capacity Rewards
Purpose: Compensate workers for maintaining available GPU resources

Measured by:

Uptime (worker health heartbeats)
VRAM availability snapshots

Formula:
R_capacity(worker) ∝ (uptime_score × available_VRAM)
Why it matters: Incentivizes maintaining a reliable, always-available network of compute resources.

Compute Rewards
Purpose: Pay workers for actual inference work performed

Measured by:

Verified output tokens generated
Model difficulty multiplier

Formula:
R_compute(worker) ∝ (total_verified_tokens × model_difficulty_multiplier)
Why it matters: Ensures fair compensation based on actual work done, with harder models paying more.

Блокчейн развернуто

Описание консенуса
https://gist.github.com/daochild/75f74177e51ed8d1aaac0f437049a6fb


Связь AI и Blockchain

Т.к. Блокчейн узлы предоставляют сети вычислительные мощности, наша трейдинг модель будет иметь приоритетный доступ (master account) к ресурсам сети с возможность требовать до n% на собственные нужды.




CoinSecurities is a SaaS investment platform where users lock capital via smart contracts and grow assets with AI-managed strategies.


Goal of AI?
Predict the best investment

User role ?
Conduct computation for Decision Engine

What is our MVP ?

AI-concierge
Based on the user's query, LLM Agent will decide which module in the Decision Engine to call. Then Decision Engine will return the best investment strategy and LLM Agent will interpret it into human readable text.


User Query → LLM Analysis → Decision Engine COMPUTATION → LLM Interpretation → Response

Уровни системы:



1. Пользовательский интерфейс (веб/мобильный кабинет, API).
2. Обработчик запросов (Модуль для понимания команд).
   LLM Integration Layer
3. Модуль принятия решений (рекомендации по инвестициям, реинвестированию)
   User Profiling Service
   История поведения (если есть)
   Риск-профиль (консервативный/умеренный/агрессивный)
   Ограничения пользователя: запрещенные активы, валюты, ликвидность
   Portfolio Optimization
   Задача оптимизации
* Reinvestment Strategy Module (Optional feature)
  Автоматическое реинвестирование
  Risk Management Module
  Система предупреждений (А-ля Низкий остаток ликвидных средств, Более 50% портфеля в одном активе и тд)

4. Интеграция с блокчейном (проверка и управление заморозкой токенов, транзакции).
5. Мониторинг и аналитика (метрики поведения пользователей, отчетность о доходности).





Endpoint Structure
/api/v1
├── /auth (аутентификация)
├── /user (профиль, настройки)
├── /chat (AI-диалоги)
├── /portfolio (портфель, балансы)
├── /recommendations (инвестиционные рекомендации)
├── /transactions (история, создание транзакций)
├── /analytics (метрики, отчеты)
└── /blockchain (интеграция с блокчейном)



DO NOT SHARE THIS PART WITH PROSPECTS
How can we integrate it  with blockchain ? (subject for discussion)
We reward the users with the most accurate predictions



Option 1
Centralized AI


CENTRAL AI SERVER
(A100 GPU)
↓
Prediction: BTC will go UP
↓
USERS decide: agree or disagree?

User A: Agrees → Stakes 1000 SECCO  on YES
User B: Disagrees → Stakes 500 SECCO  on NO
User C: Agrees → Stakes 800 SECCO  on YES  
↓
BLOCKCHAIN
↓
Smart Contract:
- Collects all predictions
- Waits for actual result
- Rewards most accurate users







Option 2
Decentralized AI
┌─────────────┐
│  User A     │ →		 Runs AI locally → Prediction: BTC +5%
│  (his GPU)  │ → 		 Stakes 1000 SECCO tokens on prediction
└─────────────┘
↓
┌─────────────┐
│  User B     │ → 		Runs AI locally → Prediction: BTC +3%
│  (his GPU)  │ → 		Stakes 500 tokens on prediction
└─────────────┘
↓
┌─────────────┐
│  User C     │ → 		Runs AI locally → Prediction: BTC -2%
│  (his GPU)  │ → 		Stakes 800 tokens on prediction
└─────────────┘
↓
BLOCKCHAIN
↓
Smart Contract:
- Collects all predictions
- Waits for actual result
- Rewards most accurate users


Option 3
Federated learning (Usually Centralized  but can be Decentralized  as well)
We use the resources of each user for computation. 



