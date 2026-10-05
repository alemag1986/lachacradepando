# La Chacra de Pando: From Telegram Messages to WoodShop Orders with Gemma

*This is a submission for the [Hacktoberfest Weekend Challenge: Build for a Friend](https://dev.to/challenges/hacktoberfest-weekend-2026-10-01)*

## What I Built

I built **La Chacra de Pando**, a Spanish-language order-management application for **the family**, who runs a small furniture workshop.

The problem I wanted to solve was practical: a order request contains much more than “make a table.” It includes the customer, dimensions, wood, finish, delivery date, and reference photos. Those details need to remain connected throughout production and delivery.

The app turns a Telegram message into a structured order without requiring the workshop owner to fill out a long form.

For example:

```text
pedido
Cliente: Ana López
Una mesa de roble, 120 × 80 × 75 cm.
Acabado natural.
```

The bot extracts the customer and product details, saves the order, and replies with a confirmation. Reference photos can accompany the request.

The website provides:

- A dashboard showing new, in-production, ready, delivered, and overdue orders.
- Searchable orders with expandable details.
- Customer contact information and order history.
- Production tasks, status changes, and promised delivery dates.
- Reference photos, delivery records, and an audit history.

Delivery is recorded with a simple Telegram command:

```text
entrega 12
```

That marks order 12 as delivered. Photos sent by the same user in the same chat during the following 15 minutes become delivery evidence.

The interface is entirely in Spanish. Bot access is restricted to an explicit Telegram user-ID whitelist.

## Demo

**Deployment address:** `https://lachacradepando.magnati.dev`

**Current status:** [Replace with the verified live link once deployment and DNS are complete, or insert a video-demo link.]

The demonstration follows one order from beginning to end:

1. Send a Spanish furniture request through Telegram.
2. Receive the bot’s order confirmation.
3. Open the order on the website and inspect its extracted details.
4. Add a production task and update the order’s status.
5. Record delivery through Telegram and attach a proof photo.

The production website requires authentication. Judges should use request access to the demo instance.

## Code

**Repository:** [Insert public GitHub repository URL.]

The application uses Rust, Axum, SQLx, SQLite, Askama, and HTMX. The repository includes database migrations, Docker deployment configuration, backup/restore scripts, and automated tests.

The recorded verification includes **39 Rust tests and four backup/archive tests**. AI integration tests use fixtures and mock services rather than making live inference calls.

## How I Built It

### Gemma: turning requests into structured data

The primary extraction model is **Gemma 4 26B A4B IT**, configured as:

```text
google/gemma-4-26b-a4b-it
```

Gemma receives the order text and, when supplied, reference images. It returns JSON containing the customer, order description, requested delivery date, and individual products—including quantity, dimensions, units, wood species, finish, and notes. [Model information](https://openrouter.ai/google/gemma-4-26b-a4b-it).

The prompt explicitly requires unknown information to remain `null`. The model must not invent prices, choose database customer IDs, or generate production tasks.

Rust validates the response before saving anything. Invalid JSON, missing contract keys, unsupported units, invalid dates, and nonpositive quantities are rejected.

Gemma interprets the request; application code controls the business rules.

### OpenRouter: hosted inference with a bounded fallback

The backend calls OpenRouter’s `/api/v1/chat/completions` endpoint using a server-side API key. Text and reference images are included in the request.

Each extraction is limited to 4,000 input characters, four images, and 800 output tokens, with temperature set to zero.

If the primary attempt fails, the application makes one fallback attempt using `qwen/qwen3.5-35b-a3b`. There are at most two inference attempts per extraction—not an unlimited retry loop.

Delivery commands do not invoke AI. Successfully committed Telegram updates are tracked so retries do not create duplicate orders or unnecessarily repeat inference.

### DigitalOcean: hosting the workshop application

The prepared deployment targets an **Ubuntu DigitalOcean Droplet with 2 GB RAM and one vCPU**, using a $12/month plan.

Docker Compose runs two services:

- **The Rust application:** Telegram webhook handling, order management, SQLite access, and photo storage.
- **Caddy:** public HTTPS, certificate management, and username/password protection for the website.

SQLite and photos persist in a mounted data directory rather than inside a disposable container. Backup scripts create a consistent SQLite snapshot, include photos, verify checksums, and retain seven daily and four weekly backups.

**Gemma does not run on the Droplet.** The Droplet hosts the application; model inference is accessed through OpenRouter.

## Why Does Open Innovation Matter?

Open-weight AI makes the interpretation layer replaceable.

The workshop’s data model and business rules belong to the application—not to a proprietary assistant conversation. Gemma produces a defined JSON contract, which the backend validates and stores in an ordinary SQLite database.

That separation means I can change models, compare extraction quality, or explore self-hosted inference later without redesigning the workshop workflow.

The current version still uses hosted inference: it needs internet access, and order text and reference images are sent to the inference service. I am not claiming offline operation or that all customer data stays on the Droplet.

The benefit is practical flexibility: an open-weight model handles the messy human request, while a small, understandable application remains responsible for orders, permissions, history, and delivery.

## My Agent Session

I used AI coding assistance to build and verify the application.

**[Optional: insert a reviewed, redacted session link. Do not upload the raw conversation—it contains a previously exposed credential.]**

## Prize Categories

- **Best Use of Gemma:** structured extraction of furniture orders from Telegram text and reference images.
- **Best Use of DigitalOcean:** hosting the application, authenticated website, webhook endpoint, and persistent workshop data. **[Include after the deployment is completed and verified.]**

OpenRouter supplies the inference integration, but it is not listed as a prize category for this challenge.
