# WOODSHOP — CODEX EXECUTION SPEC

## 1. Goal

Build and deploy a minimal production system for a wood/rustic-furniture shop.

Interfaces:

1. Telegram = all operational input.
2. Web dashboard = view/manage customers, orders, production, deliveries and delivery photos.

Infrastructure:

- DigitalOcean Droplet
- Rust
- Axum
- Askama
- HTMX
- SQLite
- Caddy HTTPS
- Docker Compose
- OpenRouter
- Local image storage
- GitHub Actions CI

Do NOT introduce React, Node backend, Kubernetes, Redis, microservices or message queues.

---

# 2. Codex orchestration

Root coordinator:

```text
model: gpt-6.1-sol
reasoning: medium
```

Responsibilities:

- architecture
- PLAN.md
- delegation
- integration
- conflict resolution
- final deployment
- final acceptance

Do not implement large components directly when they can be delegated.

Maximum concurrent subagents: 3.

## Subagents

### A — Database/domain

```text
model: gpt-6-luna
reasoning: low
owns:
  migrations/
  src/domain/
  src/db/
```

Deliver:

- SQLite schema
- SQLx repository layer
- domain models
- migrations
- DB tests

---

### B — Telegram + AI

```text
model: gpt-6-luna
reasoning: medium
owns:
  src/telegram/
  src/ai/
```

Deliver:

- Telegram webhook
- command parser
- pedido workflow
- entrega workflow
- photo handling
- OpenRouter client
- structured order extraction

---

### C — Web UI

```text
model: gpt-6-luna
reasoning: low
owns:
  src/web/
  templates/
  static/
```

Deliver:

- dashboard
- orders
- order detail
- customers
- customer detail
- deliveries
- delivery photos

Use server-rendered Askama + HTMX only.

---

### D — Infrastructure

Start after core application compiles.

```text
model: gpt-6-luna
reasoning: low
owns:
  Dockerfile
  docker-compose.yml
  Caddyfile
  scripts/
  .github/
  README.md
```

Deliver:

- production container
- persistent volumes
- Caddy TLS
- backup/restore
- CI
- deployment documentation

---

### E — QA/Security

Start after integration.

```text
model: gpt-6-luna
reasoning: medium
```

Do not rewrite architecture.

Review:

- auth
- Telegram authorization
- webhook verification
- SQL injection
- file upload validation
- path traversal
- secrets
- transaction integrity
- duplicate customers
- AI hallucination handling
- tests

Return only defects and patches.

Escalate to root Sol only for architecture/integration failures.

---

# 3. Token policy

Mandatory.

```text
1. Default subagent = gpt-6-luna low.
2. Use medium only for Telegram/AI/integration/debugging.
3. Never use Sol for routine coding.
4. No nested subagents unless root explicitly authorizes.
5. Spawn agents only for independent work.
6. Give subagents only requirements + owned files.
7. Do not send full conversation history.
8. Read targeted files; never repeatedly scan entire repository.
9. Use rg/find before opening files.
10. Run targeted tests before full test suite.
11. Status responses <= 300 words.
12. Do not explain obvious code.
13. Do not generate duplicate design documents.
14. After 2 failed attempts, escalate problem to root.
15. Root integrates; subagents do not modify files owned by other agents.
```

---

# 4. Repository

```text
/
├── AGENTS.md
├── PLAN.md
├── .agent/
│   └── PLANS.md
├── Cargo.toml
├── Cargo.lock
├── .env.example
├── .gitignore
├── Dockerfile
├── docker-compose.yml
├── Caddyfile
├── README.md
│
├── migrations/
│
├── src/
│   ├── main.rs
│   ├── config.rs
│   ├── error.rs
│   ├── db/
│   ├── domain/
│   ├── telegram/
│   ├── ai/
│   └── web/
│
├── templates/
├── static/
│   ├── htmx.min.js
│   └── app.css
│
├── tests/
│   ├── telegram.rs
│   ├── orders.rs
│   ├── deliveries.rs
│   └── fixtures/
│
└── scripts/
    ├── deploy.sh
    ├── backup.sh
    └── restore.sh
```

---

# 5. Database

Use SQLite.

Production location:

```text
/data/woodshop.db
```

Enable:

```text
WAL
foreign_keys=ON
busy_timeout=5000
```

Tables:

```text
customers
orders
order_items
work_items
deliveries
attachments
telegram_users
telegram_sessions
order_events
```

## customers

```text
id
name
phone
email
address
preferred_language
notes
created_at
updated_at
```

Match customer:

```text
1. normalized phone
2. normalized email
3. normalized name
```

Never let AI decide customer IDs.

---

## orders

```text
id
customer_id
status
original_text
normalized_description
requested_delivery_date
promised_delivery_date
created_by
created_at
updated_at
```

Status:

```text
NEW
CONFIRMED
IN_PRODUCTION
READY
DELIVERED
CANCELLED
```

---

## order_items

```text
id
order_id
product_type
description
quantity
length
width
height
dimension_unit
wood_species
finish
notes
```

Unknown values MUST remain NULL.

Never infer missing dimensions/materials.

---

## work_items

```text
id
order_id
description
status
sequence
assigned_to
created_at
completed_at
```

---

## deliveries

```text
id
order_id
delivered_at
delivered_by
notes
```

---

## attachments

```text
id
order_id
delivery_id
type
path
telegram_file_id
telegram_message_id
mime_type
created_at
```

Types:

```text
ORDER_REFERENCE
CUSTOMER_DRAWING
WORK_IN_PROGRESS
FINISHED_PRODUCT
DELIVERY_PROOF
OTHER
```

Files:

```text
/data/photos/{order_id}/
```

Do not store image binary in SQLite.

---

# 6. Telegram

Only authorized Telegram users/chats can modify data.

Environment:

```text
TELEGRAM_BOT_TOKEN
TELEGRAM_WEBHOOK_SECRET
TELEGRAM_ALLOWED_USER_IDS
```

Required commands:

## pedido

Example:

```text
pedido
Juan Perez
8325551212
Mesa rustica de roble 84x40 acabado oscuro.
Necesita entrega noviembre 15.
```

May include photos.

Flow:

```text
Telegram
→ authenticate sender
→ detect pedido
→ download reference photos
→ OpenRouter
→ validate JSON
→ resolve/create customer
→ create order transaction
→ attach photos
→ reply with order summary + ID
```

Bot response:

```text
Pedido #1042 creado
Cliente: Juan Perez
Producto: Mesa
Material: Roble
Medidas: 84x40
Entrega: Nov 15
```

---

## entrega

Example:

```text
entrega 1042
```

Flow:

```text
validate order
→ create delivery
→ status DELIVERED
→ establish delivery session for Telegram user
→ following photos attach as DELIVERY_PROOF
```

Delivery session lifetime:

```text
15 minutes
```

No LLM call.

Reply:

```text
Pedido #1042 marcado como entregado.
Las fotos enviadas durante los próximos 15 minutos se adjuntarán a esta entrega.
```

---

# 7. AI

OpenRouter.

Production:

```text
OPENROUTER_MODEL=google/gemma-4-26b-a4b-it
```

Optional development:

```text
google/gemma-4-26b-a4b-it:free
```

Fallback:

```text
qwen/qwen3.5-35b-a3b
```

AI is called ONLY for `pedido`.

Never call AI for:

```text
entrega
dashboard
customer lookup
status changes
database queries
photo storage
```

Maximum:

```text
text: 4000 chars
reference photos: 4
image longest edge: 1600px
output: ~800 tokens
retries: 1
```

Validate output using Rust types/Serde.

If invalid:

```text
retry once with validation error
```

If still invalid:

```text
store request as failed
notify Telegram user
do not create order
```

---

# 8. AI output contract

Create Rust equivalent of:

```json
{
  "customer": {
    "name": "string|null",
    "phone": "string|null",
    "email": "string|null",
    "address": "string|null",
    "preferred_language": "es|en|null",
    "notes": "string|null"
  },
  "order": {
    "description": "string",
    "requested_delivery_date": "YYYY-MM-DD|null",
    "items": [
      {
        "product_type": "string|null",
        "description": "string",
        "quantity": 1,
        "length": null,
        "width": null,
        "height": null,
        "dimension_unit": "in|ft|cm|mm|null",
        "wood_species": "string|null",
        "finish": "string|null",
        "notes": "string|null"
      }
    ]
  },
  "missing_fields": []
}
```

System prompt rules:

```text
Extract only information present in the message/images.
Translate meaning but preserve customer intent.
Normalize quantities and measurements.
Never invent missing information.
Return null when unknown.
Do not calculate price.
Do not select customer IDs.
Do not create production tasks.
Return JSON only.
```

Store:

```text
original Telegram text
AI structured result
model name
created timestamp
```

---

# 9. Web application

Pages:

```text
/
 /orders
 /orders/{id}
 /customers
 /customers/{id}
 /deliveries
```

Dashboard:

```text
NEW
IN PRODUCTION
READY
DELIVERED THIS WEEK
OVERDUE
```

Order list filters:

```text
status
customer
delivery date
search
```

Order detail shows:

```text
customer
items
dimensions
materials
finish
notes
work items
reference photos
delivery
delivery photos
event history
```

Customer page:

```text
contact details
notes
all orders
delivery history
```

Use:

```text
Askama
HTMX
plain CSS
```

No SPA.

---

# 10. Web security

Use Caddy Basic Auth for MVP.

Axum listens internally only.

Expose through Caddy:

```text
443
```

Telegram webhook endpoint must use Telegram webhook secret validation.

Apply:

```text
request size limits
image MIME whitelist
random stored filenames
no user supplied filesystem paths
HTML escaping
parameterized SQL only
```

---

# 11. Docker

Two containers only:

```text
woodshop
caddy
```

Persistent mount:

```text
./data:/data
```

No database container.

---

# 12. Environment

`.env.example`

```text
APP_ENV=production
APP_PORT=8080

DATABASE_URL=sqlite:/data/woodshop.db

TELEGRAM_BOT_TOKEN=
TELEGRAM_WEBHOOK_SECRET=
TELEGRAM_ALLOWED_USER_IDS=

OPENROUTER_API_KEY=
OPENROUTER_MODEL=google/gemma-4-26b-a4b-it
OPENROUTER_FALLBACK_MODEL=qwen/qwen3.5-35b-a3b

PUBLIC_BASE_URL=https://shop.example.com

CADDY_ADMIN_USER=
CADDY_ADMIN_PASSWORD_HASH=

RUST_LOG=info
```

Never commit `.env`.

---

# 13. Tests

Required automated tests:

```text
pedido parser
entrega parser
unauthorized Telegram user
AI JSON validation
customer exact phone match
duplicate customer prevention
order transaction rollback
delivery transition
delivery photo attachment
invalid order ID
unsupported MIME
path traversal attempt
web order list
web customer detail
```

Use fixtures for OpenRouter responses.

CI MUST NOT call OpenRouter.

---

# 14. Deployment

Target:

```text
Ubuntu DigitalOcean Droplet
Docker
Docker Compose
Caddy
```

Deployment:

```text
1. Create Droplet.
2. Configure DNS A record.
3. Install Docker.
4. Clone repository.
5. Create /opt/woodshop.
6. Create production .env.
7. docker compose build.
8. docker compose up -d.
9. Run migrations automatically at startup.
10. Register Telegram webhook.
11. Verify /health.
12. Send test pedido.
13. Send test entrega.
14. Verify delivery photo in dashboard.
```

---

# 15. Backups

`scripts/backup.sh`

Backup:

```text
SQLite database
/data/photos
```

Use SQLite online backup, not raw copy during writes.

Keep:

```text
7 daily
4 weekly
```

Design script so remote S3-compatible destination can be enabled later.

---

# 16. CI

GitHub Actions:

```text
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```

No deployment until tests pass.

---

# 17. Execution order

## Phase 0 — Root

Create:

```text
AGENTS.md
.agent/PLANS.md
PLAN.md
Cargo.toml
repository skeleton
```

Commit:

```text
chore: initialize woodshop application
```

## Phase 1 — parallel

Spawn:

```text
A database/domain
B telegram/AI
C web UI
```

Each agent commits only owned files.

## Phase 2 — root integration

Root:

```text
merge work
compile
resolve interfaces
run tests
```

## Phase 3

Spawn infrastructure agent.

## Phase 4

Run complete application locally using Docker Compose.

## Phase 5

Spawn QA/security agent.

Fix critical/high defects.

## Phase 6

Root runs:

```text
cargo fmt
cargo clippy -- -D warnings
cargo test
docker compose build
docker compose up
integration smoke tests
```

## Phase 7

Deploy DigitalOcean.

---

# 18. Definition of done

Do not declare completion until all succeed:

```text
[ ] pedido creates customer/order
[ ] existing customer is reused
[ ] Spanish input works
[ ] English input works
[ ] pedido photos stored
[ ] AI missing values remain null
[ ] entrega updates correct order
[ ] delivery photos stored
[ ] unauthorized Telegram users rejected
[ ] dashboard lists orders
[ ] customer history works
[ ] delivery history works
[ ] images display
[ ] persistence survives container restart
[ ] backup succeeds
[ ] restore succeeds
[ ] HTTPS works
[ ] Telegram webhook works publicly
[ ] cargo test passes
[ ] cargo clippy passes
[ ] production health endpoint passes
```

---

# 19. Root-agent instruction

Execute this plan autonomously.

Do not ask for implementation preferences already specified here.

When information such as domain name, DigitalOcean IP, Telegram token, OpenRouter key or passwords is unavailable:

```text
implement using environment variables
continue all other work
document the exact missing deployment value
```

Maintain `PLAN.md` as the source of truth.

Delegate independent components.

Minimize context transfer and model usage.

Prefer working code and tests over explanations