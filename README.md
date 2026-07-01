# Elyze Vote Backend

Backend de vote ultra-léger écrit en Rust. Conçu pour absorber des dizaines de milliers de requêtes par minute sur du matériel minimal.

**Architecture** : les votes sont persistés en SQLite (mode WAL), les compteurs par candidat sont mis en cache en RAM et servis sans aucun accès disque en lecture.

---

## Référence API

### POST `/vote` — Voter, changer ou annuler un vote

L'endpoint gère automatiquement les trois cas :
- **Nouveau vote** → `"voted"`
- **Même candidat à nouveau** → `"unvoted"` (toggle)
- **Candidat différent** → `"changed"`

```bash
curl -X POST http://VOTRE_SERVEUR:3000/vote \
  -H "Content-Type: application/json" \
  -d '{
    "phone_id": "device_abc123",
    "candidate_id": "candidat_42",
    "token": "VOTRE_TOKEN_HMAC"
  }'
```

**Réponse `200 OK` :**
```json
{ "status": "voted" }
{ "status": "unvoted" }
{ "status": "changed" }
```

**Réponses d'erreur :**
```json
{ "error": "invalid token" }        // 401 — HMAC invalide ou expiré
{ "error": "unknown candidate" }    // 404 — candidat non enregistré
{ "error": "missing fields" }       // 400 — champs manquants
```

---

### GET `/votes` — Récupérer tous les compteurs de votes

Servi entièrement depuis la RAM. Zéro accès à la base de données.

```bash
curl http://VOTRE_SERVEUR:3000/votes
```

**Réponse `200 OK` :**
```json
{
  "candidates": [
    { "id": "candidat_42", "count": 1234 },
    { "id": "candidat_07", "count": 987 }
  ]
}
```

---

### GET `/votes/:phone_id` — Récupérer le vote d'un téléphone

Retourne le candidat pour lequel ce téléphone a voté, ou `null` si aucun vote.

```bash
curl http://VOTRE_SERVEUR:3000/votes/device_abc123
```

**Réponse `200 OK` :**
```json
{ "candidate_id": "candidat_42" }
{ "candidate_id": null }
```

---

### POST `/candidates` — Ajouter un candidat *(admin)*

```bash
curl -X POST http://VOTRE_SERVEUR:3000/candidates \
  -H "Content-Type: application/json" \
  -H "X-Admin-Token: VOTRE_ADMIN_TOKEN" \
  -d '{ "id": "candidat_42" }'
```

**Réponse `201 Created` :**
```json
{ "status": "created" }
```

Retourne `200 { "status": "already_exists" }` si l'ID existe déjà.

---

### DELETE `/candidates/:id` — Supprimer un candidat *(admin)*

Supprime le candidat et tous ses votes associés (cascade).

```bash
curl -X DELETE http://VOTRE_SERVEUR:3000/candidates/candidat_42 \
  -H "X-Admin-Token: VOTRE_ADMIN_TOKEN"
```

**Réponse `200 OK` :**
```json
{ "status": "deleted" }
```

---

## Génération du token (HMAC)

L'app mobile envoie un token prouvant que la requête provient d'un build légitime.

**Formule :** `HMAC-SHA256(phone_id + ":" + AAAAMMJJ, VOTE_HMAC_SECRET)`

Les tokens sont valides pour aujourd'hui et hier (tolérance de fuseau horaire).

**Générer un token pour tester (bash) :**
```bash
SECRET="votre_vote_hmac_secret"
PHONE_ID="device_abc123"
DATE=$(date +%Y%m%d)

TOKEN=$(printf '%s:%s' "$PHONE_ID" "$DATE" \
  | openssl dgst -sha256 -hmac "$SECRET" \
  | awk '{print $2}')

echo "$TOKEN"
```

**Vote de test complet avec token généré :**
```bash
SECRET="votre_vote_hmac_secret"
PHONE_ID="device_abc123"
DATE=$(date +%Y%m%d)
TOKEN=$(printf '%s:%s' "$PHONE_ID" "$DATE" | openssl dgst -sha256 -hmac "$SECRET" | awk '{print $2}')

curl -X POST http://VOTRE_SERVEUR:3000/vote \
  -H "Content-Type: application/json" \
  -d "{\"phone_id\":\"$PHONE_ID\",\"candidate_id\":\"candidat_42\",\"token\":\"$TOKEN\"}"
```

**App mobile (pseudocode) :**
```
secret = BuildConfig.HMAC_SECRET   // depuis local.properties (gitignorée)
date   = aujourdhui().format("AAAAMMJJ")
token  = HMAC_SHA256(phone_id + ":" + date, secret)
```

> En développement local, mettre `VOTE_HMAC_SECRET=DISABLED` pour désactiver la validation.

---

## Déploiement sur OVH

### 1. Prérequis sur votre machine locale

```bash
# Installer Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Ajouter la cible musl Linux pour un binaire statique
rustup target add x86_64-unknown-linux-musl

# macOS uniquement : installer le cross-compilateur musl
brew install FiloSottile/musl-cross/musl-cross
```

### 2. Compiler un binaire statique

```bash
CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER=x86_64-linux-musl-gcc \
  cargo build --release --target x86_64-unknown-linux-musl

# Binaire : target/x86_64-unknown-linux-musl/release/elyze-vote
```

> **Alternative — compiler directement sur le serveur** (plus simple, pas de cross-compilation) :
> ```bash
> # Sur le serveur OVH
> curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
> source "$HOME/.cargo/env"
> git clone https://github.com/VOTRE_ORG/elyze-backend.git
> cd elyze-backend
> cargo build --release
> # Binaire : target/release/elyze-vote
> ```

### 3. Copier les fichiers sur le serveur

```bash
ssh user@VOTRE_SERVEUR "sudo mkdir -p /opt/elyze && sudo chown user:user /opt/elyze"

scp target/x86_64-unknown-linux-musl/release/elyze-vote user@VOTRE_SERVEUR:/opt/elyze/

ssh user@VOTRE_SERVEUR "chmod +x /opt/elyze/elyze-vote"
```

### 4. Créer le fichier `.env` sur le serveur

```bash
ssh user@VOTRE_SERVEUR
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

echo "=== SAUVEGARDER CES VALEURS ==="
echo "HMAC_SECRET: $HMAC_SECRET"
echo "ADMIN_TOKEN: $ADMIN_TOKEN"
```

**Notez les secrets affichés — vous ne les reverrez plus.**

### 5. Service systemd

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

### 6. Consulter les logs

```bash
journalctl -u elyze-vote -f
```

### 7. (Optionnel) Reverse proxy Nginx

```nginx
server {
    listen 80;
    server_name votredomaine.com;

    location / {
        proxy_pass http://127.0.0.1:3000;
        proxy_http_version 1.1;
        proxy_set_header Connection "";
    }
}
```

### 8. Mettre à jour le binaire

```bash
git pull
cargo build --release
sudo systemctl restart elyze-vote
```

---

## Développement local

```bash
git clone https://github.com/VOTRE_ORG/elyze-backend.git
cd elyze-backend

cp .env.example .env
# Dans .env, configurer :
#   VOTE_HMAC_SECRET=DISABLED   ← désactive la validation du token
#   ADMIN_TOKEN=dev_token

cargo run
```

Lancer les tests :
```bash
cargo test
```

---

## Performances

| Opération | Latence | Notes |
|---|---|---|
| `GET /votes` | ~1 µs | Pure RAM, zéro DB |
| `POST /vote` | < 1 ms | Un write SQLite WAL |

- SQLite WAL : ~50k écritures/sec sur un SSD basique
- Taille du binaire : ~5 Mo strippé
- RAM au repos : < 10 Mo
- Les compteurs survivent aux redémarrages — rechargés depuis la DB au boot

---

## Sécurité

- **Token HMAC** : chaque vote doit inclure un token prouvant qu'il provient d'un build signé. Le secret n'est jamais dans le code source — injecté à l'exécution via variable d'environnement.
- **Token admin** : la gestion des candidats nécessite le header `X-Admin-Token`.
- **Aucun secret dans le code** : `.env` est gitignorée. Les contributeurs génèrent leurs propres clés depuis `.env.example`.
- **Injection SQL** : toutes les requêtes utilisent des bindings paramétrés (`sqlx`).

---

## Licence

Open source. Voir LICENSE.
