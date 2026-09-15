# Revue de sécurité — Elyze_backend

Analyse manuelle du backend Rust (Axum + SQLite) : `src/main.rs`, `src/auth.rs`,
`src/db.rs`, `src/state.rs`, `src/routes/*.rs`. Revue initiale : 2026-09-15.
Correctifs appliqués : 2026-09-15 (même session).

## 🔴 Élevé

### 1. ✅ RÉSOLU — Fuite du secret du vote — `GET /votes/:phone_id` non authentifié
`src/routes/vote.rs` (`get_my_vote`)

Utilisé légitimement par l'app mobile (`VoteApi.kt`) pour que chacun consulte
**son propre** vote au lancement (restaurer l'UI). Le serveur ne vérifiait
jamais que l'appelant était bien le propriétaire du `phone_id` demandé —
aucun token requis. N'importe qui connaissant ou devinant un `phone_id`
pouvait savoir pour qui cette personne avait voté.

**Fix appliqué** : `get_my_vote` accepte maintenant un query param `token`
(même HMAC que `POST /vote`), vérifié via `auth::verify_phone_token`.
Transition en douceur : token absent/invalide → réponse `candidate_id: null`
(comme "pas voté") plutôt qu'une erreur 401, donc pas de fuite d'info et pas
de casse pour d'éventuels clients pas encore mis à jour. Aucune migration de
données nécessaire (les votes stockés ne changent pas).

Répercuté sur les deux apps mobiles (elles calculaient déjà ce token pour
voter, il ne restait qu'à l'envoyer aussi ici) :
- **Android** (`Elize2027_app`) : `VoteApi.kt` (`getDeviceVote`) et
  `VoteRepository.kt` (`fetchVotes`) — build Kotlin vérifié (`compileDebugKotlin`).
- **iOS** (`Elize2027_app_ios`) : `VoteAPI.swift` (`getDeviceVote`),
  `VoteRepository.swift` (`fetchDeviceVote`) et `VoteViewModel.swift`
  (`fetchData`) — build Xcode vérifié (`BUILD SUCCEEDED`).

⚠️ **Limite structurelle, non corrigée ici** : le secret HMAC
(`VOTE_HMAC_SECRET`) est embarqué en clair dans les deux apps pour qu'elles
puissent signer leurs votes (`Config.swift` côté iOS,
`BuildConfig.HMAC_SECRET` généré depuis `local.properties` côté Android). Un
attaquant qui décompile l'app peut extraire ce secret et donc forger un
token valide pour n'importe quel `phone_id`, ce qui contourne la protection
du point 1 pour un attaquant déterminé (pas pour un simple curieux qui
appelle l'URL à la main, ce que le fix ci-dessus bloque bien). Résoudre ça
correctement demanderait un changement d'architecture (ex : un token émis et
signé côté serveur lors d'un enregistrement d'appareil, au lieu d'un secret
partagé statique) — hors scope de cette session, à évaluer séparément si le
niveau de risque de l'app le justifie.

### 2. ✅ RÉSOLU — Comparaison du token admin non constant-time
`src/routes/candidates.rs:21` (`check_admin`)

`v == admin_token` s'arrêtait au premier octet différent → timing attack
permettant de reconstruire `ADMIN_TOKEN` octet par octet.

**Fix appliqué** : `check_admin` utilise désormais `auth::constant_time_eq`
(nouvelle fonction, comparaison XOR en temps constant, sans dépendance
externe).

## 🟠 Moyen

### 3. ✅ RÉSOLU — Comparaison HMAC non constant-time
`src/auth.rs` (`verify_phone_token`)

Même faille que le point 2, sur la vérification du token de vote
(`hex::encode(...) == token`).

**Fix appliqué** : le token est décodé en octets puis vérifié avec
`mac.verify_slice(&token_bytes)` (crate `hmac`), qui compare en temps
constant en interne.

### 4. ✅ RÉSOLU — Backdoor `"DISABLED"` sans garde-fou
`src/auth.rs`

Si `VOTE_HMAC_SECRET=DISABLED`, toute vérification de token de vote était
court-circuitée, y compris en prod en cas de mauvaise config.

**Fix appliqué** : la condition est maintenant `secret == "DISABLED" &&
cfg!(debug_assertions)`. Un `cargo build --release` (celui utilisé par ce
projet, voir `Cargo.toml`) ignore toujours `"DISABLED"`, quelle que soit la
valeur de `VOTE_HMAC_SECRET` en environnement — le bypass n'existe plus que
dans un binaire compilé en debug. Vérifié : `cargo build --release` compile
et le test `disabled_mode_always_passes` (qui tourne en profil debug/test)
passe toujours.

### 5. ✅ RÉSOLU — Pas de rate limiting
Aucune route n'avait de limitation de débit.

**Fix appliqué** : nouveau middleware maison `src/rate_limit.rs` (fenêtre
fixe en mémoire, par IP, sans dépendance externe — réutilise `dashmap` déjà
présent). Appliqué via `route_layer` : 10 req/10s/IP sur `POST /vote`, 20
req/60s/IP sur les routes admin (`/candidates`, `/candidates/:id`). Les
routes de lecture publique (`/votes`, `/votes/history`, `/votes/:phone_id`)
ne sont pas limitées (pas besoin, zéro écriture). Nécessite
`ConnectInfo<SocketAddr>` — `main.rs` sert désormais l'app via
`into_make_service_with_connect_info`.

⚠️ **Limite connue** : la détection d'IP se base sur la connexion TCP directe
(`ConnectInfo`). Si un reverse-proxy est ajouté devant ce service en
production, il faudra adapter l'extraction d'IP (ex: `X-Forwarded-For` avec
liste de proxies de confiance), sinon toutes les requêtes semblent venir de
l'IP du proxy et la limite s'applique globalement au lieu d'être par
utilisateur.

**Complément anti brute-force (2026-09-15, suite à discussion)** : ban
progressif par IP en plus de la fenêtre fixe. Après 5 échecs
d'authentification (401) consécutifs sur `/vote` ou les routes admin, l'IP
est bannie (429 `too many failed attempts`) pour une durée qui augmente à
chaque récidive : 1 min → 5 min → 30 min (palier max). Implémenté dans
`src/rate_limit.rs` (`BanState`, `record_outcome`, `currently_banned`),
nouveau champ `AppState.bans`. Vérifié dynamiquement : 5 tentatives avec
token invalide → 401, 6e → 429 avec ban ~60s, une requête suivante avec un
token *valide* reste bloquée pendant le ban. La protection réseau (anti-DDoS
OVH) reste complémentaire — elle opère en L3/L4, pas au niveau applicatif.

### 6. ✅ RÉSOLU — Pas de validation référentielle du vote
`src/routes/vote.rs` (`post_vote`)

Un utilisateur avec un token valide pouvait voter pour un `candidate_id`
arbitraire non existant, polluant les compteurs publics.

**Fix appliqué** : `post_vote` vérifie maintenant `state.counts.contains_key
(&req.candidate_id)` avant d'accepter le vote, et rejette avec 400
`unknown candidate_id` sinon.

## 🟡 Faible / info

- **✅ RÉSOLU — `phone_id` sans limite de taille** dans `VoteRequest` : ajout
  d'une limite de 128 caractères dans `post_vote` (400 `phone_id too long`
  au-delà). Le `candidate_id`, lui, est désormais borné indirectement par le
  fix du point 6 (doit correspondre à un candidat existant, donc ≤ 64
  caractères comme imposé à la création).
- **✅ RÉSOLU — `Cargo.lock` gitignoré** : la ligne a été retirée du
  `.gitignore`. À committer (`git add Cargo.lock`) pour des builds
  reproductibles et un audit de dépendances fiable — non fait automatiquement
  ici, c'est une action git laissée à l'équipe.
- **✅ RÉSOLU (partiellement) — `cargo-audit`** : installé et exécuté.
  Résultat initial : 1 vulnérabilité (medium) + 2 avertissements.
  - `event-listener 5.4.1` (RUSTSEC-2026-0221, unsound) et `spin 0.9.8`
    (yanked) — tous deux transitifs via `sqlx-sqlite`/`flume`, **réellement
    compilés** dans le binaire. **Corrigés** par `cargo update -p
    event-listener -p spin` → `event-listener 5.4.2`, `spin 0.9.9`.
    `cargo build`, `cargo build --release` et `cargo test` (6/6) repassent
    après la mise à jour.
  - `rsa 0.9.10` (RUSTSEC-2023-0071, Marvin Attack timing sidechannel,
    medium 5.9, **aucun correctif disponible en amont**) — **non résolu**,
    mais **non exploitable dans ce binaire** : c'est une dépendance
    transitive du driver MySQL de `sqlx` (`caching_sha2_password`), qui
    n'existe que comme entrée `Cargo.lock` (résolution de features pour tout
    le graphe) sans jamais être compilée, puisque ce projet n'active que la
    feature `sqlite` de `sqlx`. Vérifié : ni `rsa` ni `sqlx-mysql` n'ont de
    fichiers dans `target/debug/deps` après build. À surveiller (relancer
    `cargo audit` périodiquement, en CI) au cas où un correctif amont
    sortirait.
- **Non résolu (pas une vulnérabilité en soi)** — Pas de CORS/security
  headers configurés. L'absence de CORS est le comportement par défaut le
  plus restrictif (refuse les requêtes cross-origin depuis un navigateur) ;
  ça ne devient un problème que si le frontend doit appeler ce backend depuis
  un domaine différent, auquel cas il faudra ajouter `tower-http::cors` de
  façon explicite et restreinte (pas de wildcard `*`). Laissé tel quel faute
  de visibilité sur la topologie de déploiement réelle.

## ✅ Points positifs

- Aucune injection SQL : toutes les requêtes sont paramétrées, y compris la
  clause `IN (...)` dynamique dans `get_history` (seul le nombre de `?` est
  généré depuis l'input, jamais les valeurs).
- `.env` correctement gitignoré, `.env.example` propre sans valeurs réelles.
- Secrets chargés via variables d'environnement avec `.expect()` → échec au
  démarrage si absents (fail-closed), pas de valeur par défaut dangereuse
  pour `ADMIN_TOKEN`/`VOTE_HMAC_SECRET`.
- Aucun bloc `unsafe`.
- Endpoints admin protégés par header token (`X-Admin-Token`).

## État global

Points 1 à 9 traités (`cargo build`, `cargo build --release` et
`cargo test` passent tous, à chaque étape). Reste à faire, hors code :
- committer `Cargo.lock` maintenant qu'il n'est plus gitignoré,
- surveiller `RUSTSEC-2023-0071` (crate `rsa`, non exploitable ici mais sans
  correctif amont) — relancer `cargo audit` périodiquement,
- décider du CORS si le frontend appelle le backend depuis un autre domaine,
- si un reverse-proxy est mis en place devant le service, adapter le rate
  limiting (voir avertissement au point 5).
