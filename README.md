# Elyze Vote Backend

Ultra-lightweight voting backend built in Rust. Designed to handle tens of thousands of requests per minute on minimal hardware.

**Architecture**: votes are persisted to SQLite (WAL mode), candidate vote counts are cached in RAM and served without any DB hit on reads.

---

## API Reference

### POST `/vote` — Cast, change, or retract a vote

The endpoint handles all three cases automatically:
- **New vote** → `"voted"`
- **Same candidate again** → `"unvoted"` (toggle off)
- **Different candidate** → `"changed"`

```bash
curl -X POST http://YOUR_SERVER:3000/vote \
  -H "Content-Type: application/json" \
  -d '{
    "phone_id": "device_abc123",
    "candidate_id": "candidate_42",
    "token": "YOUR_HMAC_TOKEN"
  }'
```

**Response `200 OK`:**
```json
{ "status": "voted" }
{ "status": "unvoted" }
{ "status": "changed" }
```

**Error responses:**
```json
{ "error": "invalid token" }        // 401 — bad or expired HMAC
{ "error": "unknown candidate" }    // 404 — candidate not registered
{ "error": "missing fields" }       // 400
```

---

### GET `/votes` — Get all vote counts

Served entirely from RAM. Zero DB hit.

```bash
curl http://YOUR_SERVER:3000/votes
```

**Response `200 OK`:**
```json
{
  "candidates": [
    { "id": "candidate_42", "count": 1234 },
    { "id": "candidate_07", "count": 987 }
  ]
}
```

---

### POST `/candidates` — Register a candidate *(admin)*

```bash
curl -X POST http://YOUR_SERVER:3000/candidates \
  -H "Content-Type: application/json" \
  -H "X-Admin-Token: YOUR_ADMIN_TOKEN" \
  -d '{ "id": "candidate_42" }'
```

**Response `201 Created`:**
```json
{ "status": "created" }
```

Returns `200 { "status": "already_exists" }` if the ID already exists.

---

### DELETE `/candidates/:id` — Remove a candidate *(admin)*

Deletes the candidate and all associated votes (cascade).

```bash
curl -X DELETE http://YOUR_SERVER:3000/candidates/candidate_42 \
  -H "X-Admin-Token: YOUR_ADMIN_TOKEN"
```

**Response `200 OK`:**
```json
{ "status": "deleted" }
```

---

## Token Generation (HMAC)

The mobile app sends a token proving the request comes from a legitimate app install.

**Formula:** `HMAC-SHA256(phone_id + ":" + YYYYMMDD, VOTE_HMAC_SECRET)`

Tokens are valid for today and yesterday (timezone tolerance).

**Generate a token for testing (bash):**
```bash
SECRET="your_vote_hmac_secret"
PHONE_ID="device_abc123"
DATE=$(date +%Y%m%d)

TOKEN=$(printf '%s:%s' "$PHONE_ID" "$DATE" \
  | openssl dgst -sha256 -hmac "$SECRET" \
  | awk '{print $2}')

echo "$TOKEN"
```

**Full test vote with generated token:**
```bash
SECRET="your_vote_hmac_secret"
PHONE_ID="device_abc123"
DATE=$(date +%Y%m%d)
TOKEN=$(printf '%s:%s' "$PHONE_ID" "$DATE" | openssl dgst -sha256 -hmac "$SECRET" | awk '{print $2}')

curl -X POST http://YOUR_SERVER:3000/vote \
  -H "Content-Type: application/json" \
  -d "{\"phone_id\":\"$PHONE_ID\",\"candidate_id\":\"candidate_42\",\"token\":\"$TOKEN\"}"
```

**Mobile app (pseudocode):**
```
secret = BuildConfig.HMAC_SECRET   // from gitignored local.properties
date   = today().format("YYYYMMDD")
token  = HMAC_SHA256(phone_id + ":" + date, secret)
```

> During local development, set `VOTE_HMAC_SECRET=DISABLED` to skip validation entirely.

---

## Deploy to OVH

### 1. Prerequisites on your local machine

```bash
# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Add the Linux musl target for a fully static binary
rustup target add x86_64-unknown-linux-musl

# macOS only: install the musl cross-linker
brew install FiloSottile/musl-cross/musl-cross
```

### 2. Build a static binary

```bash
CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER=x86_64-linux-musl-gcc \
  cargo build --release --target x86_64-unknown-linux-musl

# Binary: target/x86_64-unknown-linux-musl/release/elyze-vote
```

> **Alternative — compile directly on the server** (simpler, no cross-compilation needed):
> ```bash
> # On the OVH server
> curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
> source "$HOME/.cargo/env"
> git clone https://github.com/YOUR_ORG/elyze-backend.git
> cd elyze-backend
> cargo build --release
> # Binary: target/release/elyze-vote
> ```

### 3. Copy files to the server

```bash
ssh user@YOUR_SERVER "sudo mkdir -p /opt/elyze && sudo chown user:user /opt/elyze"

scp target/x86_64-unknown-linux-musl/release/elyze-vote user@YOUR_SERVER:/opt/elyze/

ssh user@YOUR_SERVER "chmod +x /opt/elyze/elyze-vote"
```

### 4. Create the `.env` on the server

```bash
ssh user@YOUR_SERVER
cd /opt/elyze

HMAC_SECRET=$(openssl rand -hex 32)
ADMIN_TOKEN=$(openssl rand -hex 24)

cat > .env << EOF
VOTE_HMAC_SECRET=${HMAC_SECRET}
ADMIN_TOKEN=${ADMIN_TOKEN}
DATABASE_URL=sqlite:///opt/elyze/votes.db
LISTEN_ADDR=0.0.0.0:3000
RUST_LOG=info
EOF

chmod 600 .env

echo "=== SAVE THESE ==="
echo "HMAC_SECRET: $HMAC_SECRET"
echo "ADMIN_TOKEN: $ADMIN_TOKEN"
```

**Save the printed secrets somewhere safe — you won't see them again.**

### 5. systemd service

```bash
sudo tee /etc/systemd/system/elyze-vote.service > /dev/null << 'EOF'
[Unit]
Description=Elyze Vote Backend
After=network.target

[Service]
Type=simple
User=www-data
Group=www-data
WorkingDirectory=/opt/elyze
EnvironmentFile=/opt/elyze/.env
ExecStart=/opt/elyze/elyze-vote
Restart=always
RestartSec=5
MemoryMax=256M
CPUQuota=80%
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ReadWritePaths=/opt/elyze

[Install]
WantedBy=multi-user.target
EOF

sudo systemctl daemon-reload
sudo systemctl enable elyze-vote
sudo systemctl start elyze-vote
sudo systemctl status elyze-vote
```

### 6. View logs

```bash
journalctl -u elyze-vote -f
```

### 7. (Optional) Nginx reverse proxy

```nginx
server {
    listen 80;
    server_name yourdomain.com;

    location / {
        proxy_pass http://127.0.0.1:3000;
        proxy_http_version 1.1;
        proxy_set_header Connection "";
    }
}
```

### 8. Update the binary

```bash
# Build locally, copy, restart
scp target/x86_64-unknown-linux-musl/release/elyze-vote user@YOUR_SERVER:/opt/elyze/elyze-vote
ssh user@YOUR_SERVER "sudo systemctl restart elyze-vote"
```

---

## Local Development

```bash
git clone https://github.com/YOUR_ORG/elyze-backend.git
cd elyze-backend

cp .env.example .env
# In .env, set:
#   VOTE_HMAC_SECRET=DISABLED   ← skips token validation
#   ADMIN_TOKEN=dev_token

cargo run
```

Run tests:
```bash
cargo test
```

---

## Performance

| Operation | Latency | Notes |
|---|---|---|
| `GET /votes` | ~1 µs | Pure RAM, no DB |
| `POST /vote` | < 1 ms | One SQLite WAL write |

- SQLite WAL: ~50k writes/sec on a basic SSD
- Binary size: ~5 MB stripped
- Idle RAM: < 10 MB
- Counts survive restarts — reloaded from DB on boot

---

## Security

- **HMAC token**: every vote must include a token proving it originates from a signed app build. The secret is never in source — injected at runtime via env var.
- **Admin token**: candidate management requires `X-Admin-Token` header.
- **No secrets in source**: `.env` is gitignored. Contributors generate their own keys from `.env.example`.
- **SQL injection**: all queries use parameterized bindings (`sqlx`).

---

## License

Open source. See LICENSE.
