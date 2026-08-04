# Security audit: Sarca (Tauri v2 client + Axum server)

Дата: 2026-08-03. Ветка `master`, база `025d4d0`.
Область: `client/src-tauri` (Rust + Tauri v2), `client/src` и `ui/src` (JS/Solid), `sarca` (Axum-сервер), `crates/sarca-sync`, CI.

Статус проверок после исправлений:

```
task lint                 -> clean (nightly rustfmt --check + clippy -D warnings, pedantic/nursery/cargo)
cargo test -p sarca --lib -> 121 passed
cargo test --lib (client) -> 68 passed
vitest run (ui)           -> 26 files, 165 tests passed
OSV npm (ui + client)     -> no advisories
```

---

## 1. Таблица находок

| # | Найдено | Риск | Файл | Исправление |
|---|---------|------|------|-------------|
| 1 | `capabilities/default.json` содержал `remote.urls: ["http://*:*/*", "https://*:*/*"]` — любой HTTP(S)-сайт, открытый в WebView, получал `__TAURI_INTERNALS__` и весь список команд | Critical | `client/src-tauri/capabilities/default.json` | Блок `remote` удалён, файл стал `"local": true`. Доступ выдаётся в рантайме через `remote_ipc::grant_remote_capability`, ограниченно origin'ом сервера, к которому подключился пользователь |
| 2 | Кастомный протокол `sarca-ipc` отвечал `Access-Control-Allow-Origin: *` и не проверял ни origin запроса, ни URL страницы — любой сайт вызывал нативные команды | Critical | `client/src-tauri/src/remote_ipc.rs` | `authorize_request()`: origin обязателен и сверяется с `is_trusted_ipc_origin`, плюс проверяется URL самого webview. ACAO — конкретный origin + `Vary: Origin`. Не-POST отвергается, тело ограничено 12 MiB |
| 3 | IPC навигации (`on_navigation`) не проверял, с какой страницы пришёл вызов | Critical | `client/src-tauri/src/remote_ipc.rs` | Тот же `is_trusted_ipc_url` перед обработкой; недоверенная страница получает отказ |
| 4 | `add_binding` / `update_binding_local_path` принимали произвольный путь: можно было забиндить `~/.ssh`, `~/.gnupg` или каталог данных самого клиента и выгрузить его на сервер | Critical | `client/src-tauri/src/commands.rs` | Новый модуль `client/src-tauri/src/paths.rs`: `validate_local_dir()` канонизирует путь, требует нахождения внутри allowed roots, режет dot-каталоги, служебные каталоги платформы и собственные data/config/cache каталоги приложения |
| 5 | `"csp": null` в конфиге Tauri — CSP не выставлялась вообще | High | `client/src-tauri/tauri.conf.json` | Полная CSP без `unsafe-inline`/`unsafe-eval` в `script-src`; `object-src`/`frame-src`/`frame-ancestors`/`base-uri`/`form-action` закрыты |
| 6 | PIN блокировки приложения хранился открытым текстом, отдавался фронтенду в `get_client_prefs` и сравнивался в JS | High | `client/src-tauri/src/state.rs`, `commands.rs`, `ui/src/components/AppLockGate.jsx` | PIN солится (UUIDv4) и хэшируется SHA-256 в 120 000 раундов. Наружу уходит `ClientPrefsDto` только с флагом `app_lock_pin_set`. Сравнение — новая команда `verify_app_lock_pin` в Rust, constant-time |
| 7 | Legacy-PIN не мигрировал: serde-ключ старого поля не совпадал, старый открытый PIN оставался в файле | High | `client/src-tauri/src/state.rs` | `#[serde(rename = "app_lock_pin", skip_serializing)] legacy_app_lock_pin` + `migrate_legacy_pin()`: при первой загрузке пересчитывается в хэш, открытое поле стирается |
| 8 | Токены сессии и PIN писались с umask по умолчанию (обычно 0644) — читаемы другими пользователями машины | High | `client/src-tauri/src/state.rs` | `write_private()`: `OpenOptions::mode(0o600)` + `set_permissions(0o600)` для уже существующих файлов |
| 9 | CORS на `/api` был `allow_origin(Any) + allow_headers(Any)` — любой сайт читал ответы API, включая `/api/setup` на свежем инстансе | High | `sarca/src/server.rs` | `cors_layer()`: только origin'ы Tauri-оболочки + явный список из `SARCA_CORS_ORIGINS`; методы и заголовки перечислены поимённо |
| 10 | Никакого throttling на неаутентифицированных сравнениях секретов: `/api/auth/login`, `/api/auth/password/forgot`, `/api/public/shares/{token}/unlock` | High | `sarca/src/common/throttle.rs` (новый), `routers/auth.rs`, `routers/public_shares.rs` | `FailureThrottle`: 5 бесплатных попыток, дальше экспоненциальная задержка до 4 с, после 25 неудач — 429; распад за 15 минут, карта ограничена 4096 ключами |
| 11 | Живые секреты в рабочем `sarca.conf`: `SUPERUSER_PASS`, `SECRET_KEY`, `DEBUG_LOG=1` | High | `sarca.conf` (в `.gitignore`, в истории git отсутствует) | Утечки в репозиторий нет. **Требуется ротация вручную**, см. раздел 4 |
| 12 | `solid-js` 1.8.6 — XSS через JSX-фрагменты (GHSA-3qxh-p7jc-5xh6) + транзитивный `seroval` (Critical) | High | `ui/package.json` | 1.8.6 -> 1.9.14 |
| 13 | Отсутствовали CSP / X-Frame-Options / nosniff / Referrer-Policy на ответах SPA | Medium | `sarca/src/server.rs` | `with_security_headers()` навешивает 5 заголовков на весь роутер |
| 14 | Пользовательские файлы отдавались с исполняемым `Content-Type` — HTML-файл в хранилище выполнялся в origin приложения | Medium | `sarca/src/routers/files.rs` | `is_active_content()` + `apply_user_content_headers()`: `Content-Disposition: attachment`, `nosniff`, изолирующая CSP на активном контенте |
| 15 | `jsonwebtoken` 9.3.1 — GHSA-h395-gr6q-cpjc (claim type confusion пропускал валидацию) | Medium | `sarca/Cargo.toml`, `sarca/src/common/jwt_manager.rs` | 10.4.0 с единственным крипто-провайдером `aws_lc_rs`; общая `validation()` фиксирует HS256 и требует `exp` и `sub` |
| 16 | `reqwest` 0.11 тянул `rustls-webpki` 0.101.7 — 6 advisories, включая panic-DoS на разборе CRL | Medium | `sarca/Cargo.toml` | reqwest 0.12 (rustls-tls) -> webpki 0.103.13 |
| 17 | Dev-server advisories: vite 4.5.0 / 5.4.21, vitest 2.1.9, транзитивные postcss/rollup/esbuild | Medium | `ui/package.json`, `client/package.json` | vite 7.3.6, vitest 3.2.7, vite-plugin-solid 2.11.14 |
| 18 | Кэш превью рос без ограничений (локальный DoS по диску) | Medium | `client/src-tauri/src/commands.rs` | `MAX_PREVIEW_BYTES` 8 MiB, `MAX_PREVIEW_B64_LEN`, `MAX_CACHE_KEY_LEN` 4096; лимит кэша зажат между 16 MiB и 64 GiB |
| 19 | Workflows CI без блока `permissions:` — наследовали дефолт репозитория, который может быть write | Medium | `.github/workflows/ui.yml`, `client.yml` | `permissions: contents: read` (ни один job не пишет в репозиторий) |
| 20 | Isolation Pattern не включён; updater отсутствует, поэтому проверки подписи нет | Medium | `client/src-tauri/tauri.conf.json` | Не менялось намеренно, см. раздел 3 |
| 21 | Токен в query-параметре принимался на любом маршруте | Medium | `sarca/src/common/routing/middlewares/auth.rs` | `is_media_get()`: `?access_token=` работает только для GET `…/files/{download,thumb,preview}`; всё остальное требует заголовок |
| 22 | JWT нельзя было отозвать: смена пароля и logout не инвалидировали уже выданные токены | Medium | `sarca/src/repositories/users.rs`, `middlewares/auth.rs` | `users.sessions_valid_after` сравнивается с `iat` токена |
| 23 | `SMTP_TLS=none` молча использовал `builder_dangerous` — пароль SMTP и ссылки сброса уходили в открытом виде | Low | `sarca/src/common/mailer.rs` | Предупреждение в лог, когда `SMTP_TLS=none` сочетается с заданными учётными данными |
| 24 | Внешние строки в `client/src/sync.js` попадали в `innerHTML` | Low | `client/src/sync.js`, `ui/src/common/sanitizeHtml.js` | `escapeHtml()` на всех подстановках; отдельный `sanitizeHtml` для UI |
| 25 | Actions запинены по тегу (`@v4`/`@v5`), а не по SHA | Low | `.github/workflows/*.yml` | Не менялось; при желании — pin по SHA, см. раздел 3 |

Что проверено и оказалось чистым: `unsafe` в клиентском коде нет (`unsafe_code = "forbid"` в `sarca`, в `client/src-tauri` — ни одного блока); ни одного `Command::new` / вызова shell; SQL идёт только через `sqlx` с bind-параметрами; `eval` / `new Function` / `document.write` в JS отсутствуют; HTML-предпросмотр уже открывался в `<iframe sandbox="">`; токены верификации почты и сброса пароля — `Uuid::new_v4()` (122 бита) под SHA-256, брутфорс нерелевантен; `route_layer(logged_in_required)` в `StoragesRouter` действительно покрывает вложенные `nest()`-роутеры (проверено по исходникам axum 0.8.9, `path_router.rs:311-341`) — обхода авторизации нет.

---

## 2. Диффы

### 2.1 Модель разрешений Tauri (пункты 1, 3)

```diff
--- a/client/src-tauri/capabilities/default.json
+++ b/client/src-tauri/capabilities/default.json
-  "description": "Default Sarca client capabilities (local shell + remote server UI)",
+  "description": "Local shell only. Remote pages get no ACL from this file: the Sarca server the user connects to is granted a narrower, origin-scoped capability at runtime (see remote_ipc::grant_remote_capability), and everything else reaches the bridge through nothing at all.",
+  "local": true,
   "windows": ["main"],
-  "remote": {
-    "urls": [
-      "http://*:*/*",
-      "https://*:*/*"
-    ]
-  },
   "permissions": [
```

Взамен — capability, выдаваемая в рантайме ровно одному origin:

```rust
// client/src-tauri/src/remote_ipc.rs
pub fn grant_remote_capability(app: &AppHandle, origin: &str) -> Result<(), String> {
    if origin.is_empty() {
        return Ok(());
    }
    let mut builder = tauri::ipc::CapabilityBuilder::new(format!("remote-server:{origin}"))
        .local(false)
        .remote(origin_url_pattern(origin))
        .window("main");
    for cmd in REMOTE_SETTINGS_COMMANDS {
        builder = builder.permission(permission_for(cmd));
    }
    app.add_capability(builder).map_err(|e| e.to_string())
}
```

Регрессия закрыта тестом в `acl_check.rs`, который падает, если блок `remote` вернётся в файл:

```rust
assert!(
    cap.get("remote").is_none(),
    "capabilities/default.json must not carry a `remote` block: it would \
     hand __TAURI_INTERNALS__ to pages the WebView loads. ..."
);
```

### 2.2 IPC-граница: origin и страница (пункты 2, 3)

```diff
--- a/client/src-tauri/src/remote_ipc.rs
+++ b/client/src-tauri/src/remote_ipc.rs
-fn cors_response(status: StatusCode, body: Vec<u8>, content_type: &str) -> Response<Vec<u8>> {
+fn ipc_response(status: StatusCode, body: Vec<u8>, content_type: &str, origin: &str)
+    -> Response<Vec<u8>> {
     ...
-        .header("Access-Control-Allow-Origin", "*")
-        .header("Access-Control-Allow-Methods", "GET, POST, OPTIONS")
-        .header("Access-Control-Allow-Headers", "Content-Type, Authorization, X-Requested-With")
-        .header("Access-Control-Max-Age", "86400")
+        .header("Access-Control-Allow-Origin", origin)
+        .header("Vary", "Origin")
+        .header("Access-Control-Allow-Methods", "POST, OPTIONS")
+        .header("Access-Control-Allow-Headers", "Content-Type")
+        .header("Access-Control-Max-Age", "600")
+        .header("Cache-Control", "no-store")
```

```rust
fn authorize_request(
    app: &AppHandle,
    request: &Request<Vec<u8>>,
    webview_label: &str,
) -> Option<String> {
    let state = app.try_state::<AppSyncState>()?;

    let origin = request
        .headers()
        .get("Origin")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|o| !o.is_empty())?;

    if !state.is_trusted_ipc_origin(origin) {
        tracing::warn!(origin, "native IPC refused: untrusted origin");
        return None;
    }

    // Origin можно подделать вне браузера, поэтому дополнительно проверяем,
    // что сама страница в webview - доверенная.
    let page_ok = app
        .get_webview_window(webview_label)
        .and_then(|wv| wv.url().ok())
        .is_some_and(|url| state.is_trusted_ipc_url(&url));
    if !page_ok {
        tracing::warn!(origin, webview_label, "native IPC refused: untrusted page");
        return None;
    }

    Some(origin.to_owned())
}
```

Метод и размер тела:

```rust
if request.method() != tauri::http::Method::POST { /* 405 */ }
if request.body().len() > MAX_IPC_BODY_BYTES { /* 413 */ }   // 12 MiB
```

### 2.3 Path traversal в биндингах синхронизации (пункт 4)

Новый `client/src-tauri/src/paths.rs`:

```rust
/// Canonicalize `raw` and confirm it is a directory the user may sync.
///
/// * `allowed_roots` - the user's own file roots (home, Android shared storage).
///   An empty list means "no root could be resolved", which is refused rather
///   than treated as "allow everything".
/// * `denied_roots` - application-owned directories (the client's own data /
///   config / cache dirs). Binding one of those would upload the session tokens
///   and the local sync database.
pub fn validate_local_dir(
    raw: &str,
    allowed_roots: &[PathBuf],
    denied_roots: &[PathBuf],
) -> Result<String, String>
```

Отсекаются: путь длиннее 4096, любой компонент-точка (`.ssh`, `.gnupg`, `.aws`, `.config`, профили браузеров — денилист по именам всегда был бы неполным), служебные каталоги платформы, каталоги самого приложения. Вызывается из обеих команд:

```diff
-                commands::update_binding_local_path(state.clone(), id, local_path).await?;
+                commands::update_binding_local_path(app.clone(), state.clone(), id, local_path)
+                    .await?;
```

### 2.4 CSP в Tauri (пункт 5)

```diff
     "security": {
-      "csp": null
+      "csp": {
+        "default-src": "'self'",
+        "script-src": "'self'",
+        "style-src": "'self'",
+        "img-src": "'self' data: blob: asset: http://asset.localhost https://asset.localhost",
+        "font-src": "'self' data:",
+        "connect-src": "'self' ipc: http://ipc.localhost sarca-ipc: http://sarca-ipc.localhost",
+        "media-src": "'self' data: blob: asset: http://asset.localhost https://asset.localhost",
+        "worker-src": "'self' blob:",
+        "frame-src": "'none'",
+        "object-src": "'none'",
+        "base-uri": "'none'",
+        "form-action": "'none'",
+        "frame-ancestors": "'none'"
+      },
+      "dangerousDisableAssetCspModification": false,
+      "capabilities": ["default"]
     }
```

Ни `unsafe-inline`, ни `unsafe-eval` в `script-src` нет.

### 2.5 PIN блокировки приложения (пункты 6, 7)

```rust
// client/src-tauri/src/state.rs
const MIN_PIN_LEN: usize = 4;
const MAX_PIN_LEN: usize = 64;
const PIN_HASH_ROUNDS: u32 = 120_000;

pub struct ClientPrefs {
    pub app_lock_pin_salt: Option<String>,
    pub app_lock_pin_hash: Option<String>,
    /// Старое открытое поле. Читается один раз ради миграции и никогда не пишется.
    #[serde(rename = "app_lock_pin", default, skip_serializing)]
    pub legacy_app_lock_pin: Option<String>,
    ...
}

/// То, что видит фронтенд. Хэша и соли здесь нет.
pub struct ClientPrefsDto {
    pub app_lock_pin_set: bool,
    ...
}
```

Сравнение переехало в Rust:

```rust
#[tauri::command]
pub fn verify_app_lock_pin(state: State<'_, AppSyncState>, pin: String) -> Result<bool, String> {
    let prefs = load_prefs(&state);
    if !prefs.app_lock_enabled || !prefs.has_pin() {
        return Ok(true);
    }
    Ok(prefs.verify_pin(&pin))   // constant_time_eq внутри
}
```

Миграция запускается при загрузке настроек, старое поле стирается с диска:

```diff
+    if prefs.migrate_legacy_pin() {
+        // перезаписываем файл уже без открытого PIN
     }
```

### 2.6 Права на файлы состояния (пункт 8)

```rust
pub fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    #[cfg(unix)]
    {
        let mut file = fs::OpenOptions::new()
            .write(true).create(true).truncate(true)
            .mode(0o600)
            .open(path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        // `mode` действует только при создании; чиним уже существующий файл,
        // записанный прошлой версией с обычным umask.
        fs::set_permissions(path, PermissionsExt::from_mode(0o600))?;
        return Ok(());
    }
    // Windows: каталог AppData уже ACL'ится на пользователя.
}
```

### 2.7 CORS и заголовки безопасности сервера (пункты 9, 13)

```diff
-        let app_cors = cors::CorsLayer::new()
-            .allow_methods(cors::Any)
-            .allow_headers(cors::Any)
-            .allow_origin(cors::Any);
+        let app_cors = cors_layer();
```

```rust
const SHELL_ORIGINS: &[&str] =
    &["tauri://localhost", "http://tauri.localhost", "https://tauri.localhost"];
const CORS_ORIGINS_VAR: &str = "SARCA_CORS_ORIGINS";

fn cors_layer() -> cors::CorsLayer {
    let layer = cors::CorsLayer::new()
        .allow_methods([Method::GET, Method::HEAD, Method::POST, Method::PUT,
                        Method::PATCH, Method::DELETE, Method::OPTIONS])
        .allow_headers([ACCEPT, AUTHORIZATION, CONTENT_TYPE, RANGE,
                        IF_MODIFIED_SINCE, IF_NONE_MATCH])
        .max_age(std::time::Duration::from_mins(10));

    let configured = std::env::var(CORS_ORIGINS_VAR).unwrap_or_default();
    let configured: Vec<&str> =
        configured.split(',').map(str::trim).filter(|o| !o.is_empty()).collect();

    if configured.contains(&"*") {
        tracing::warn!(
            "{CORS_ORIGINS_VAR} contains '*': every website can read API responses. \
             List explicit origins instead."
        );
        return layer.allow_origin(cors::Any);
    }

    let mut origins: Vec<HeaderValue> = Vec::new();
    for origin in SHELL_ORIGINS.iter().copied().chain(configured) {
        if let Ok(value) = HeaderValue::from_str(origin) {
            origins.push(value);
        } else {
            tracing::warn!(origin, "ignoring unparsable {CORS_ORIGINS_VAR} entry");
        }
    }
    layer.allow_origin(origins)
}
```

```rust
fn with_security_headers(router: Router) -> Router {
    // `style-src` нужен 'unsafe-inline': SUID вставляет стили компонентов
    // как inline <style> в рантайме. Скрипты остаются только 'self' -
    // именно это и блокирует XSS.
    const CSP: &str = "default-src 'self'; \
         script-src 'self'; \
         style-src 'self' 'unsafe-inline' https://fonts.googleapis.com; \
         font-src 'self' data: https://fonts.gstatic.com; \
         img-src 'self' data: blob:; \
         media-src 'self' data: blob:; \
         connect-src 'self'; \
         worker-src 'self' blob:; \
         frame-src 'none'; \
         object-src 'none'; \
         base-uri 'self'; \
         form-action 'self'; \
         frame-ancestors 'none'";

    let headers: [(HeaderName, &'static str); 5] = [
        (HeaderName::from_static("content-security-policy"), CSP),
        (HeaderName::from_static("x-content-type-options"), "nosniff"),
        (HeaderName::from_static("x-frame-options"), "DENY"),
        (HeaderName::from_static("referrer-policy"), "strict-origin-when-cross-origin"),
        (HeaderName::from_static("permissions-policy"),
         "accelerometer=(), camera=(), geolocation=(), gyroscope=(), magnetometer=(), \
          microphone=(), payment=(), usb=()"),
    ];

    headers.into_iter().fold(router, |router, (name, value)| {
        router.layer(SetResponseHeaderLayer::overriding(name, HeaderValue::from_static(value)))
    })
}
```

### 2.8 Throttling неаутентифицированных попыток (пункт 10)

Новый `sarca/src/common/throttle.rs`. Ключ — то, что атакуют (адрес, токен ссылки), а не IP: сервер стоит за TLS-терминацией и HTTP/3-путями, где `ConnectInfo` есть не везде, а IP всё равно тривиально ротируется.

```rust
const FREE_ATTEMPTS: u32 = 5;      // столько попыток бесплатны
const LOCK_ATTEMPTS: u32 = 25;     // после стольких - отказ без сравнения
const MAX_DELAY: Duration = Duration::from_secs(4);
const DECAY: Duration = Duration::from_mins(15);
const MAX_KEYS: usize = 4096;      // карта ограничена

pub async fn check(&self, key: &str) -> SarcaResult<()> {
    match self.decide(key) {
        Decision::Locked => Err(SarcaError::TooManyAttempts),
        Decision::Delay(delay) => {
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            Ok(())
        },
    }
}

fn delay_for(failures: u32) -> Duration {
    if failures < FREE_ATTEMPTS {
        return Duration::ZERO;
    }
    let steps = failures - FREE_ATTEMPTS;
    let millis = 250_u64.saturating_mul(1_u64 << steps.min(6));
    Duration::from_millis(millis).min(MAX_DELAY)
}
```

Общий счётчик живёт в `AppState`, так что клонирование состояния между хендлерами не сбрасывает его:

```diff
 pub struct AppState {
     pub db: Pool<Sqlite>,
     pub config: Config,
     pub tx: ClientSender,
+    /// Общий тормоз для всех неаутентифицированных сравнений секретов
+    /// (login, разблокировка ссылки, письмо сброса). Клонирование
+    /// `AppState` сохраняет те же счётчики.
+    pub throttle: FailureThrottle,
 }
```

Логин:

```diff
 pub async fn login(...) -> SarcaResult<(StatusCode, Json<TokenSchema>)> {
-    let schema = AuthService::new(&state.db).login(login_data, &state.config).await?;
-    Ok((StatusCode::OK, Json(schema)))
+    let key = keys::login(&login_data.email);
+    state.throttle.check(&key).await?;
+    match AuthService::new(&state.db).login(login_data, &state.config).await {
+        Ok(schema) => {
+            state.throttle.record_success(&key);
+            Ok((StatusCode::OK, Json(schema)))
+        },
+        Err(e) => {
+            state.throttle.record_failure(&key);
+            Err(e.into())
+        },
+    }
 }
```

Запрос сброса пароля (ответ по-прежнему всегда 204, чтобы не давать перечисление аккаунтов):

```rust
let key = keys::forgot_password(&body.email);
if state.throttle.check(&key).await.is_ok() {
    state.throttle.record_failure(&key);
    AuthService::new(&state.db).forgot_password(&body.email, &state.config).await;
}
StatusCode::NO_CONTENT
```

Разблокировка публичной ссылки:

```rust
let throttle_key = keys::share_unlock(&token);
state.throttle.check(&throttle_key).await.map_err(err_response)?;
...
PublicSharesService::verify_password(&link, &body.password).map_err(|e| {
    state.throttle.record_failure(&throttle_key);
    ...
})?;
state.throttle.record_success(&throttle_key);
```

Отдельный код ответа, чтобы 429 не смешивался с 401:

```diff
+    #[error("too many attempts, try again later")]
+    TooManyAttempts,
...
+    SarcaError::TooManyAttempts => (StatusCode::TOO_MANY_REQUESTS, e.to_string()),
```

Покрытие: 8 тестов в `throttle.rs` (бесплатные попытки, рост и потолок задержки, блокировка, сброс после успеха, распад, независимость ключей, изоляция пространств имён, ограниченность карты) + 2 в `errors.rs`.

### 2.9 JWT (пункт 15)

```diff
-jsonwebtoken = "9"
+# 10.x fixes GHSA-h395-gr6q-cpjc (claim type confusion skipped validation).
+# Exactly one crypto provider may be on, or every sign/verify panics; aws_lc_rs
+# is already in the tree via rustls, and picking it keeps the `rsa` crate
+# (RUSTSEC-2023-0071, Marvin) out of the build entirely. Only HS256 is used.
+jsonwebtoken = { version = "10", default-features = false, features = ["aws_lc_rs"] }
```

```rust
/// HS256 only, with `exp` and `sub` demanded rather than merely checked.
///
/// Naming the algorithm keeps a token whose header says `alg: none` (or
/// any asymmetric algorithm) from being accepted, and requiring the claims
/// means a token that simply omits `exp` cannot slip through as
/// "never expires".
fn validation() -> Validation {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.set_required_spec_claims(&["exp", "sub"]);
    validation
}
```

7 тестов: round-trip, чужой секрет, истёкший токен, невзаимозаменяемость access/refresh, `alg: none`, cookie разблокировки только для своей ссылки, access-токен не подходит как cookie разблокировки.

### 2.10 Область действия токена в query (пункт 21)

```rust
fn is_media_get(req: &Request<axum::body::Body>) -> bool {
    if req.method() != Method::GET {
        return false;
    }
    let path = req.uri().path();
    let Some((_, rest)) = path.split_once("/files/") else {
        return false;
    };
    let action = rest.split('/').next().unwrap_or_default();
    matches!(action, "download" | "thumb" | "preview")
}
```

`?access_token=` принимается только там, где `<video>`/`<img>` не могут послать заголовок. Покрыто тестами `media_gets_accept_a_query_token` и `other_routes_and_methods_do_not`.

### 2.11 Пользовательский контент с исполняемым типом (пункт 14)

```rust
let active = is_active_content(&content_type);
apply_user_content_headers(headers_mut, active);
```

HTML/SVG/XML из хранилища отдаётся как вложение, с `nosniff` и собственной запирающей CSP, поэтому не выполняется в origin приложения.

### 2.12 Отзыв сессий (пункт 22)

```rust
if !user.session_is_live(auth_user.issued_at) {
    return Err(<(StatusCode, String)>::from(SarcaError::NotAuthenticated));
}
```

`users.sessions_valid_after` двигается при смене пароля и при logout, поэтому ранее выданные stateless-токены перестают работать.

### 2.13 SMTP в открытом виде (пункт 23)

```diff
             "none" => {
+                if self.config.smtp_password.is_some() {
+                    tracing::warn!(
+                        "SMTP_TLS=none with SMTP credentials set: the SMTP password and every \
+                         reset link are sent to {host} in clear text"
+                    );
+                }
                 AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(host)
                     .port(self.config.smtp_port)
             },
```

### 2.14 Зависимости (пункты 12, 16, 17)

| Пакет | Было | Стало | Почему |
|-------|------|-------|--------|
| `solid-js` | 1.8.6 | 1.9.14 | GHSA-3qxh-p7jc-5xh6 (XSS в JSX-фрагментах) + `seroval` Critical |
| `vite` (ui) | 4.5.0 | 7.3.6 | advisories dev-сервера, транзитивные esbuild/rollup/postcss |
| `vite` (client) | 5.4.21 | 7.3.6 | то же |
| `vitest` | 2.1.9 | 3.2.7 | совместимость с vite 7 + advisories |
| `vite-plugin-solid` | 2.7.2 | 2.11.14 | совместимость с solid 1.9 |
| `jsonwebtoken` | 9.3.1 | 10.4.0 | GHSA-h395-gr6q-cpjc |
| `reqwest` | 0.11 | 0.12 | вытягивал `rustls-webpki` 0.101.7 (6 advisories, включая panic-DoS на CRL) |

`cargo audit` и `pnpm audit` в системе не установлены, поэтому проверка шла через batch-запросы к OSV.dev (`https://api.osv.dev/v1/querybatch`) по `Cargo.lock` и обоим `pnpm-lock.yaml`. Скрипт: `scratchpad/osv_audit.py`.

### 2.15 CI (пункт 19)

```diff
+# Least privilege: these jobs build and test, they never write to the repo.
+# Artifacts are uploaded through actions/upload-artifact, which does not need
+# `contents: write`.
+permissions:
+  contents: read
+
 concurrency:
```

Проверено grep'ом: ни `ui.yml`, ни `client.yml` не делают push, не создают релизы и не пишут комментарии.

---

## 3. Остаточный риск: что осталось и почему

**Advisories в crates.io, которые нельзя закрыть обновлением.**

| Крейт | ID | Почему остаётся |
|-------|-----|-----------------|
| `atk`, `gdk*`, `gtk*` 0.18.2 | RUSTSEC-2024-0411…0420 | Привязки GTK3, объявлены unmaintained. Тянутся Tauri на Linux, апстрим-замены нет. Не уязвимость, а статус сопровождения |
| `glib` 0.18.5 | GHSA-wrw7-89jp-8q8g / RUSTSEC-2024-0429 | Unsoundness в `VariantStrIter`. Тот же GTK3-стек; код Sarca этот итератор не вызывает |
| `event-listener` 5.4.1 | RUSTSEC-2026-0221 | `!Send`-теги через `StackSlot`. Транзитивный, исправленной версии на момент аудита нет |
| `rsa` 0.9.10 | RUSTSEC-2023-0071 (Marvin) | Только в `Cargo.lock` через **неиспользуемый** optional-feature `sqlx-mysql`. В сборку не попадает: `cargo tree -i rsa -e normal` печатает «nothing to print» |
| `proc-macro-error`, `rustls-pemfile`, `unic-*` | RUSTSEC-2024-0370, 2025-0134, 2025-0075/0080/0081/0098/0100 | Unmaintained, не уязвимости |

**Isolation Pattern (пункт 7 чек-листа).** Не включён. Он даёт смысл, когда фронтенд может содержать сторонний код; здесь фронтенд собирается из собственных исходников, а `script-src 'self'` без `unsafe-inline` уже блокирует инъекцию. Включение потребует отдельного бандла изоляции и переработки IPC-пути `sarca-ipc`. Решение стоит принимать отдельно.

**Updater.** В `tauri.conf.json` плагина обновлений нет вообще, поэтому «не отключена ли проверка подписи» неприменимо: обновлений нет. Если добавлять — сначала `tauri signer generate`, приватный ключ в секреты CI, публичный в `plugins.updater.pubkey`, и `"createUpdaterArtifacts": true`.

**Подпись сборок.** macOS собирается с `signingIdentity: "-"` — ad-hoc подпись, годится для CI-артефактов, не для распространения. Настоящий Developer ID + нотаризация, а также Windows Authenticode — решение уровня релизного пайплайна, требует ключей.

**Pin actions по SHA.** `actions/checkout@v4`, `setup-node@v4`, `pnpm/action-setup@v4`, `dtolnay/rust-toolchain@stable`, `Swatinem/rust-cache@v2`, `android-actions/setup-android@v4`, `actions/setup-java@v5` запинены тегами. Тег можно передвинуть; SHA нельзя. Смена дешёвая, но ломает автообновления Dependabot без дополнительной настройки.

**Токены в браузерном хранилище.** Access/refresh лежат в storage, а не в `HttpOnly` cookie. Это стандартный компромисс SPA; после включения CSP без `unsafe-inline` вектор их кражи существенно сузился. Полный переход на cookie потребует CSRF-защиты на всех мутирующих маршрутах.

**`FluentIcon`** имеет проп `src`, который попадает в raw innerHTML. Вызовов с этим пропом сейчас нет, но это скрытая поверхность — стоит либо удалить проп, либо прогонять через `sanitizeHtml`.

> **Поправка (раунд 2).** Утверждение «вызовов с этим пропом сейчас нет» было
> неверным: `FilesSidebar.jsx` использовал `src` в пяти местах. Проп удалён,
> вызовы переведены на `name`. См. раунд 2, находка R2-4.

---

## 4. Что нужно сделать вручную

1. **Ротировать секреты в рабочем `sarca.conf`.** Файл в `.gitignore`, в истории git его нет (проверено `git log --all -- sarca.conf` и `git grep` по истории) — утечки наружу не было. Но значения реальные:
   - `SUPERUSER_PASS` — сменить пароль суперпользователя;
   - `SECRET_KEY` — перегенерировать; это инвалидирует все выданные JWT, включая cookie разблокировки ссылок (ожидаемо и желательно);
   - `DEBUG_LOG=1` — выключить на продакшене.
2. **Проверить, что новая CSP сервера не ломает SPA.** Собрать `task ui`, открыть приложение и посмотреть консоль на предмет CSP-нарушений. Особое внимание: рантайм-инъекция стилей SUID, шрифты Google, и скрипт инъекции сессии Tauri (он выполняется на уровне движка, не через тег `<script>`, поэтому под CSP попадать не должен).
3. **Проверить клиент на реальном сервере после сужения ACL.** Убедиться, что удалённая страница сервера по-прежнему получает нужные команды настроек через `grant_remote_capability`, а сторонняя страница — нет.
4. **Установить `cargo audit` и `pnpm audit` в CI.** Сейчас проверка одноразовая, через скрипт OSV. Постоянный job в CI + Dependabot/Renovate закроют дрейф.
5. **Пентест сетевого API.** За пределами статического аудита: авторизация между пользователями (IDOR по `storage_id` / `file_id` / токенам ссылок), гонки на квотах и в загрузке, поведение на границах Range-запросов, устойчивость при массовой параллельной загрузке.
6. **Проверить настройки SMTP на продакшене.** `SMTP_TLS` должен быть `starttls` или `tls`; при `none` сервер теперь предупреждает в логе, но не отказывается работать.
7. **Решить по Isolation Pattern, updater и подписи релизов** — см. раздел 3.
8. **Проверить права на уже существующие файлы состояния клиента.** `write_private` чинит их при следующей записи, но пока запись не произошла, старый файл остаётся с прежним режимом. `chmod 600` на каталог данных клиента вручную ускорит это.

---
---

# Раунд 2

Повторный аудит по тому же чек-listу, база `ca6bf0f`. Находки раунда 1 не
перепроверялись построчно и здесь не повторяются — ниже только то, что раунд 1
пропустил или описал неверно.

## R2.1 Таблица находок

| # | Найдено | Риск | Файл | Исправление |
|---|---------|------|------|-------------|
| R2-1 | Приватный ключ подписи Android лежит в репозитории (`sarca-sideload.p12`, пароль `sarca-sideload` в открытом виде в скриптах), и релизный пайплайн **молча откатывался на него**, когда секреты `ANDROID_KEYSTORE_*` не заданы | **Critical** | `.github/workflows/release.yml`, `client/scripts/sign-android-apk.sh` | Релиз падает с ошибкой вместо отката. Локальный скрипт требует явного `SARCA_ALLOW_PUBLIC_KEYSTORE=1` |
| R2-2 | Адрес сервера без схемы превращался в `http://` — включая публичные хосты. `sarca.example.com` уходил по открытому HTTP вместе с access/refresh-токенами | **High** | `crates/sarca-sync/src/api.rs` | `is_local_host()`: loopback/LAN остаются на `http://`, всё маршрутизируемое получает `https://` |
| R2-3 | Origin Vite dev-сервера (`localhost:1420`) считался доверенной оболочкой **и в релизной сборке** | **High** | `client/src-tauri/src/state.rs` | Ветка закрыта `#[cfg(debug_assertions)]` |
| R2-4 | Проп `FluentIcon src` попадал в `innerHTML` без санитизации. Раунд 1 записал его как «вызовов нет» — вызовы были, 5 штук | **Medium** | `ui/src/components/FluentIcon.jsx`, `ui/src/components/FilesSidebar.jsx` | Проп удалён; вызовы переведены на `name`; поиск по таблице через `Object.hasOwn` |
| R2-5 | Неиспользуемая devDependency `solid-devtools` тянула уязвимый `@babel/core@7.23.3` / `@babel/helpers@7.23.4` (GHSA-968p-4wvh-cqc8, GHSA-4x5r-pxfx-6jf8) | **Medium** | `ui/package.json` | Зависимость удалена; обе уязвимые копии ушли из lock-файла |
| R2-6 | В CI не было постоянной проверки зависимостей (незакрытый пункт 4 из раздела «вручную» раунда 1) | **Medium** | `.github/workflows/audit.yml` | Новый workflow: `cargo audit` + `pnpm audit`, по PR, по push и еженедельно |
| R2-7 | `cache_get_preview` не ограничивал длину ключа, в отличие от парного `cache_put_preview` | **Low** | `client/src-tauri/src/commands.rs` | Та же проверка `MAX_CACHE_KEY_LEN` |
| R2-8 | Регрессионный guard в CI **требовал** статический wildcard `remote.urls: ["http://*:*"]` — то есть охранял ровно ту дыру, которую раунд 1 закрыл, и ронял CI за её отсутствие | **Medium** | `client/scripts/check-remote-acl.py` | Переписан: теперь запрещает статический `remote.urls` в любом capability-файле и проверяет runtime-список `REMOTE_SETTINGS_COMMANDS` в обе стороны |

## R2.2 Диффы

### R2-1. Публичный ключ подписи как запасной вариант релиза

Риск конкретный, и он не про утечку файла. Android определяет «то же самое
приложение» по подписи. Ключ и пароль лежат в репозитории, значит любой может
собрать APK, подписать им, и система примет этот APK как **обновление**
установленной Sarca — с доступом ко всем её данным. Пока `release.yml` молча
откатывался на этот ключ, достаточно было один раз выпустить релиз без
настроенных секретов, чтобы раздать пользователям сборку с публично известным
ключом. Дальше отозвать его нельзя: сменить ключ подписи можно только
переустановкой приложения вручную.

```diff
-          if [[ -n "${ANDROID_KEYSTORE_BASE64:-}" ]]; then
-            ...
-            ALIAS="${ANDROID_KEY_ALIAS:-sarca}"
-            echo "Using ANDROID_KEYSTORE_* secrets for APK signing."
-          else
-            KS_PATH="${GITHUB_WORKSPACE}/client/mobile/sarca-sideload.p12"
-            STORE_PASS="sarca-sideload"
-            KEY_PASS="sarca-sideload"
-            ALIAS="sarca"
-            echo "Using committed sideload keystore for APK signing."
-          fi
+          if [[ -z "${ANDROID_KEYSTORE_BASE64:-}" ]]; then
+            echo "::error::ANDROID_KEYSTORE_BASE64 / ANDROID_KEYSTORE_PASSWORD are not set." >&2
+            echo "Refusing to publish a release APK signed with the public committed keystore." >&2
+            echo "Generate a release key and add it to repository secrets:" >&2
+            echo "  keytool -genkeypair -v -keystore release.p12 -storetype PKCS12 \\" >&2
+            echo "    -alias sarca -keyalg RSA -keysize 4096 -validity 10000" >&2
+            echo "  base64 -w0 release.p12   # -> secret ANDROID_KEYSTORE_BASE64" >&2
+            exit 1
+          fi
+          KS_PATH="${GITHUB_WORKSPACE}/client/src-tauri/release.keystore"
+          echo "${ANDROID_KEYSTORE_BASE64}" | base64 -d > "${KS_PATH}"
+          STORE_PASS="${ANDROID_KEYSTORE_PASSWORD:?ANDROID_KEYSTORE_PASSWORD required}"
+          KEY_PASS="${ANDROID_KEY_PASSWORD:-$ANDROID_KEYSTORE_PASSWORD}"
+          ALIAS="${ANDROID_KEY_ALIAS:-sarca}"
```

`sign-android-apk.sh` теперь тоже не берёт публичный keystore сам:

```diff
-else
+elif [[ "${SARCA_ALLOW_PUBLIC_KEYSTORE:-0}" == "1" ]]; then
   KS="$DEFAULT_KS"
   ALIAS="$DEFAULT_ALIAS"
   STORE_PASS="$DEFAULT_PASS"
   KEY_PASS="$DEFAULT_PASS"
+  echo "WARNING: signing with the PUBLIC committed sideload keystore: $KS" >&2
+  echo "WARNING: the private key and password are in the repository. Never distribute this APK." >&2
+else
+  # ...инструкция по keytool + base64 -w0...
+  exit 1
 fi
```

`client.yml` (smoke-артефакт, который никогда не публикуется) явно
подтверждает согласие через `SARCA_ALLOW_PUBLIC_KEYSTORE: "1"`.

### R2-2. Неявный `http://` для публичного хоста

```diff
 fn normalize_server_url(raw: &str) -> Result<String> {
     let with_scheme = if trimmed.contains("://") {
         trimmed.to_owned()
     } else {
-        format!("http://{trimmed}")
+        // Parse once against a placeholder scheme just to isolate the host.
+        let host_only = reqwest::Url::parse(&format!("http://{trimmed}"))
+            .ok()
+            .and_then(|u| u.host_str().map(str::to_owned))
+            .unwrap_or_default();
+        if is_local_host(&host_only) {
+            format!("http://{trimmed}")
+        } else {
+            format!("https://{trimmed}")
+        }
     };
```

`is_local_host()` покрывает loopback, RFC1918, link-local, `.local`,
`.internal`, `.home.arpa`, ULA `fc00::/7` и `fe80::/10`. Самостоятельный хостинг
в локальной сети не ломается: `192.168.1.40:8001` по-прежнему `http://`. Явный
`http://sarca.example.com` тоже уважается — понижение происходило только при
неявном выборе схемы за пользователя.

### R2-3. Dev-origin в релизной сборке

`is_shell_url` определяет, какие страницы получают полный набор команд
оболочки, включая `connect`, `update_session` и `get_url_history`. Порт 1420 в
релизном бинарнике никогда не является Vite: это просто локальный порт, который
может занять любой процесс пользователя.

```diff
-            // Vite `devUrl` in tauri.conf.json (port 1420) only.
+            // Vite `devUrl` in tauri.conf.json (port 1420). Debug builds only:
+            // in a shipped binary this would hand full shell trust — and with it
+            // `connect`, `update_session` and `get_url_history` — to any local
+            // process that manages to bind port 1420 before the user browses to
+            // it. A release bundle never loads the dev server.
+            #[cfg(debug_assertions)]
             Some("localhost") | Some("127.0.0.1") => url.port() == Some(1420),
```

### R2-4. `FluentIcon`: удалён raw-innerHTML проп

```diff
 const FluentIcon = (props) => {
 	const svg = () => {
-		if (props.src) return props.src
-		if (props.name && fluentIcons[props.name]) return fluentIcons[props.name]
+		if (props.name && Object.hasOwn(fluentIcons, props.name)) {
+			return fluentIcons[props.name]
+		}
 		return ''
 	}
```

Переход на `Object.hasOwn` закрывает заодно обращение по цепочке прототипов:
`name="toString"` раньше возвращал функцию `Object.prototype.toString`, она
проходила проверку на truthy и уезжала в `innerHTML`.

Вызовы в `FilesSidebar.jsx` переведены с SVG-строк на имена:

```diff
-{item('browse', 'All files', fluentIcons.folder, fluentIcons.folderFilled)}
+{item('browse', 'All files', 'folder', 'folderFilled')}
```

Проверено статически: 88 имён во всех `name=` по `ui/src` резолвятся в таблице,
ни одного оставшегося `src=`.

### R2-5 и R2-6. Зависимости и постоянная проверка в CI

`solid-devtools` не импортировался ни в `vite.config.js`, ни в исходниках —
только висел в `devDependencies` и удерживал старый Babel. Удалён.

Новый `.github/workflows/audit.yml`: `permissions: contents: read`, запуск по
PR, push в `master`, вручную и еженедельно. Rust-часть — `cargo audit` с одним
исключением, JS-часть — `pnpm audit --audit-level=moderate` по `ui` и `client`.

### R2-7. Ограничение длины ключа кэша

```diff
 ) -> Result<Option<String>, String> {
+    // Same bound as `cache_put_preview`: the keys are hashed, so an unbounded
+    // one only buys the caller hashing work on a multi-megabyte string.
+    if scope.len() > MAX_CACHE_KEY_LEN || path.len() > MAX_CACHE_KEY_LEN {
+        return Err("preview cache key too long".into());
+    }
```

### R2-8. Guard, охранявший саму дыру

`client/scripts/check-remote-acl.py` ронял CI на `ca6bf0f`:

```
FAIL: remote.urls must include host:port wildcards (*:*); got [].
      Plain http://* does not match http://host:port/
```

Скрипт был написан под старую, статическую модель ACL и утверждал, что
`capabilities/default.json` **обязан** содержать `"remote": {"urls":
["http://*:*"]}`. Раунд 1 этот блок удалил и перенёс выдачу прав в
`grant_remote_capability`, где они привязаны к одному origin, на который
пользователь реально подключился. То есть проверка не просто устарела: она
требовала вернуть широкий грант, отдающий Sync/Security-команды любому http
origin, куда получится увести webview.

Переписан так, чтобы держать обе стороны инварианта:

```python
FORBIDDEN_REMOTE = ["connect", "get_url_history"]

# (2) No static remote grant, in this or any other capability file.
for path in sorted(CAP_DIR.glob("*.json")):
    data = json.loads(path.read_text())
    urls = (data.get("remote") or {}).get("urls") or []
    if urls:
        failures.append(
            f"{path.name} declares a static remote.urls grant {urls!r}. "
            "Remote access must be granted at runtime by "
            "grant_remote_capability, scoped to the connected origin."
        )
```

Проверяется четыре вещи: 17 нужных `allow-*` на месте; статического
`remote.urls` нет ни в одном `capabilities/*.json`; runtime-список
`REMOTE_SETTINGS_COMMANDS` (парсится регуляркой из `remote_ipc.rs`) покрывает
эти команды; `connect` и `get_url_history` в нём **отсутствуют** — иначе
удалённая страница смогла бы сама передвинуть границу доверия.

Проверено в обе стороны: возврат `"urls": ["http://*:*"]` в `default.json` даёт
`FAIL: default.json declares a static remote.urls grant [...]`, добавление
`connect` в список — `FAIL: REMOTE_SETTINGS_COMMANDS must not expose 'connect'
to remote pages`. На чистом дереве: `OK: 17 Sync/Security allows present; 30
runtime remote commands; no static remote.urls grant`.

### Побочно: порядок шагов в `e2e-gui`

Второй упавший job к безопасности отношения не имеет, но чинится тем же
заходом. `.github/workflows/e2e.yml`, job `e2e-gui`:

```
chmod: cannot access 'target/release/sarca': No such file or directory
```

Два `download-artifact` клали бинарники в `target/release` и
`target/debug/examples` **до** шага `Swatinem/rust-cache@v2`, который
восстанавливает `target/` целиком. В логе упавшего запуска: `Cache hit for
restore-key ... Cache restored successfully`. Оба шага скачивания
отработали (`Total of 1 artifact(s) downloaded` трижды), а до `chmod` файлы не
дожили. Поэтому job падал только на запусках с попаданием в кэш и выглядел
плавающим, а не сломанным по порядку.

Оба скачивания перенесены после шага кэша; `ui-dist` (в `ui/dist`, вне
`target/`) остался на месте. Порядок шагов подтверждён по логу; внутренний
механизм, которым rust-cache стирает содержимое `target/`, до конца не
разбирался — на корректность порядка это не влияет.

## R2.3 Что проверено и чем

В отличие от раунда 1, `cargo audit` и `pnpm audit` в этот раз были **запущены**,
а не только рекомендованы.

| Проверка | Команда | Результат |
|---|---|---|
| Rust advisories | `cargo audit` (v0.22.2) | 1 vulnerability, 17 unmaintained, 2 unsound |
| Достижимость `rsa` | `cargo tree -i rsa -e normal` | «nothing to print» — в сборку не попадает |
| JS advisories, до фикса | `pnpm audit` в `ui` | 1 moderate + 1 low (оба Babel, dev-only) |
| JS advisories, после | `pnpm audit` в `ui` и `client` | `No known vulnerabilities found` |
| Тесты sarca-sync | `task client:check-sync` | 55 passed |
| Тесты клиента | `cargo test -p sarca-client --lib` | 68 passed |
| Тесты UI | `pnpm test` в `ui` | 165 passed, 26 файлов |
| Сборка UI | `pnpm build` | успешно |
| Линт | `task lint` | чисто |
| Синтаксис workflow | `yaml.safe_load` + `bash -n` | чисто |
| Guard remote-ACL | `python3 client/scripts/check-remote-acl.py` | OK; негативные тесты в обе стороны падают как задумано |

Единственное исключение в `cargo audit` — `RUSTSEC-2023-0071` (Marvin, `rsa`
0.9.x, патча нет). Оно попадает в lock-файл только через отключённую
optional-фичу `sqlx-mysql` и в бинарник не идёт, что подтверждено `cargo tree`.
Все 19 остальных — `unmaintained` / `unsound`; `cargo audit` без флага `-D` на
них не падает, поэтому в список исключений они не внесены: новое
**настоящее** уязвимое место уронит job.

`task lint` покрывает только пакет `sarca`. `cargo +nightly fmt` по
`sarca-sync` и `sarca-client` показывает 8 расхождений, но они предшествуют
этим правкам — тот же результат на чистом дереве через `git stash`. Не трогал:
к безопасности отношения не имеет, а шум в диффе мешал бы ревью.

## R2.4 Осознанно не исправлено

**Android: `cleartextTrafficPermitted="true"` глобально**
(`client/mobile/android/res/xml/network_security_config.xml`). Разрешает
открытый HTTP к любому хосту, а не только к локальной сети. Правильнее было бы:

```xml
<base-config cleartextTrafficPermitted="false" />
<domain-config cleartextTrafficPermitted="true">
    <domain includeSubdomains="true">192.168.0.0</domain>
    ...
</domain-config>
```

Не применил: Android не поддерживает CIDR в `<domain>`, поэтому честного
«разрешить всю локальную сеть» здесь не выразить, а перечислять конкретные
адреса — значит сломать заявленный сценарий самостоятельного хостинга у
пользователей с произвольным адресом сервера. R2-2 уже снимает основную часть
риска: теперь до открытого HTTP дело доходит только когда пользователь либо
указал LAN-адрес, либо явно написал `http://`. Решение — за владельцем
продукта.

## R2.5 Что осталось проверить вручную

Пункты 1, 2, 3, 5, 6, 7, 8 из раздела 4 раунда 1 остаются в силе. Пункт 4
(`cargo audit` / `pnpm audit` в CI) закрыт находкой R2-6. Дополнительно:

1. **Ротировать ключ подписи Android, если релиз уже выходил без секретов.**
   Проверить по опубликованным APK: `apksigner verify --print-certs` и сравнить
   отпечаток с `sarca-sideload.p12`. Если совпал — ключ публичный, и
   пользователей нужно переводить на новый ключ переустановкой. Смена ключа
   подписи без переустановки в Android невозможна.
2. **Завести секреты `ANDROID_KEYSTORE_BASE64` и `ANDROID_KEYSTORE_PASSWORD`.**
   До этого момента релизный Android-job будет падать — это и есть задуманное
   поведение, но релиз без них теперь не соберётся.
3. **Удалить `client/mobile/sarca-sideload.p12` из репозитория и из истории git.**
   Сейчас он остаётся для локальных sideload-сборок за явным флагом. Если он не
   нужен — вычистить историю (`git filter-repo`), потому что ключ, лежавший в
   публичной истории, скомпрометирован навсегда.
4. **Пентест сетевого API** — как в раунде 1: IDOR по `storage_id` / `file_id` /
   токенам ссылок, гонки на квотах, границы Range-запросов.
5. **Проверить sidebar после R2-4 глазами.** Иконки резолвятся статически и
   тесты проходят, но подмена SVG-строк на имена — визуальное изменение;
   стоит открыть список файлов и убедиться, что все пять иконок на месте в
   обоих состояниях (обычном и активном).

---

# Раунд 3 (2026-08-04, HEAD `2584668`)

Проверка после коммитов `3268d5e`..`2584668`, которые прошли уже после раундов 1-2.

## R3.1 Таблица находок

| Найдено | Риск | Файл | Исправление |
| --- | --- | --- | --- |
| Loopback-прокси вебвью принимает любой `Host`: DNS rebinding делает страницу атакующего same-origin с `127.0.0.1:<port>` и даёт ей читать ответы и дёргать весь Sarca API | High | `crates/sarca-sync/src/proxy.rs` | Проверка `Host` на литеральный loopback-authority с нашим портом, иначе 403 (`is_loopback_host`) |
| `tauri-plugin-pilot` закреплён по `tag`, а тег мутабельный: force-push подменяет код под тем же именем | Medium | `client/src-tauri/Cargo.toml` | Пин по `rev = "a6c5baa…"` |
| Лог `ACME certificate issued (not_after=…)` удалён вместе с inline-выдачей при старте; e2e-тест на него ещё ждал — красный CI | Medium (CI/наблюдаемость) | `sarca/src/tls/renew.rs` | Логировать в ветке успеха задачи продления; `renew_once` возвращает `not_after` |
| Ветка ошибки логировала `ACME renewal failed`, а `assert_no_log("ACME issuance failed")` стал бессмысленным | Low | `sarca/src/tls/renew.rs` | Переименовано в `ACME issuance failed` |
| Запас теста 60s против фактических ~50s выдачи (VALIDATION_DELAY 35s + бэкофф) — флаки на нагруженном раннере | Low | `e2e/test_12_acme_tls.py` | Таймаут 120s |

## R3.2 Проверено и признано закрытым (без изменений)

- **CSP**: `tauri.conf.json` задаёт директивную карту без `unsafe-inline`/`unsafe-eval`;
  `object-src`/`base-uri`/`form-action`/`frame-ancestors` = `'none'`.
  `'unsafe-eval'` появляется только в `lib.rs::pilot_context` под
  `cfg(all(desktop, debug_assertions, feature = "pilot"))` — в релиз не попадает.
- **Capabilities**: единственный `default.json`, `local: true`, окно `main`,
  без wildcard; каждая команда перечислена поимённо. Удалённый origin получает
  права только через `remote_ipc::grant_remote_capability`.
- **innerHTML-сингки**: `FileViewer` (markdown, docx) проходят через
  `sanitizeHtml` (DOMPurify, FORBID_TAGS/ATTR); `client/src/sync.js` экранирует
  через `escapeHtml`; `FluentIcon` не принимает raw-markup проп.
  `eval` / `new Function` / `document.write` / `dangerouslySetInnerHTML` — нет.
- **unsafe в Rust**: блоков `unsafe` нет ни в `sarca`, ни в `client/src-tauri`,
  ни в `sarca-sync`.
- **Секреты**: в дереве только тестовые значения (`e2e-password-123`,
  `test-secret-value`, `webview-live-token`).
- **Зависимости**: `cargo audit` — 1 уязвимость, RUSTSEC-2023-0071 (`rsa` 0.9.10,
  Marvin), патча нет; приходит только через неактивную фичу `sqlx-mysql`,
  `cargo tree -i rsa -e normal` печатает "nothing to print", в бинарь не входит.
  Игнор задокументирован в `.github/workflows/audit.yml`. Остальные 19 —
  `unmaintained`/`unsound` (GTK3-биндинги, `event-listener`, `glib`).
- **Updater**: плагин обновлений не подключён вообще, поэтому проверку подписи
  отключить негде. Если его когда-нибудь включат — `pubkey` обязателен.
- **Подпись сборок**: Android релизный APK отказывается публиковаться с
  закоммиченным keystore и проверяется `apksigner`; iOS подписывается при
  наличии `APPLE_*` секретов.

## R3.3 Что осталось проверить вручную

1. **macOS**: `bundle.macOS.signingIdentity` = `"-"` (ad-hoc). Для распространения
   нужен Developer ID + нотаризация; ad-hoc бинарь Gatekeeper не пропустит.
2. **Windows**: подпись Authenticode в `release.yml` не настроена.
3. **Isolation Pattern**: не включён. Оправданно, пока весь фронтенд свой, но
   удалённая страница Sarca-сервера рендерится в том же вебвью — если модель
   угроз допускает скомпрометированный сервер, изоляцию стоит включить.
4. Пентест сетевого API сервера (authz по объектам, IDOR, rate limits) — вне
   статического анализа.
5. Ротация `ANDROID_KEYSTORE_*` / `APPLE_*` секретов в GitHub.

---

# Раунд 4 (2026-08-04, HEAD `47c9e3f`)

Раунды 1-3 закрыли весь чек-лист по клиенту (capabilities, CSP, IPC-граница,
пути, фронтенд, unsafe, зависимости, подпись). Раунд 4 смотрел туда, куда
предыдущие не доходили: на сам движок синхронизации, то есть на код, который
исполняет то, что сказал сервер, уже **после** всех проверок границы.

## R4.1 Таблица находок

| Найдено | Риск | Файл | Исправление |
| --- | --- | --- | --- |
| Скачивание писало прямо в `dest` через `tokio::fs::write`. Если по этому пути лежит симлинк, запись идёт **в цель симлинка**: скомпрометированный сервер, назвав файл `innocent.jpg`, перезаписывает `~/.bashrc` или `~/.ssh/authorized_keys`. Zip-slip (`..` в пути) раунд 1 закрыл, symlink-slip — нет | High | `crates/sarca-sync/src/api.rs` | Запись во временный файл рядом (`create_new`, то есть тоже не через ссылку) + `rename` на место. `rename` заменяет симлинк, а не идёт по нему |
| Тело ответа буферизовалось целиком: `resp.bytes().await`. Сервер отдаёт на маленькую запись снапшота ответ в несколько гигабайт — OOM клиента | Medium | `crates/sarca-sync/src/api.rs` | `MAX_DOWNLOAD_BYTES` = 16 GiB: проверка `Content-Length` до чтения и счётчик по потоку (`bytes_stream`), так как заголовок необязателен и может врать |
| Скан выгрузки резолвил симлинк-файлы через `fs::metadata` и грузил **цель**. `paths::validate_local_dir` ограничивает только корень биндинга, поэтому ссылка `~/Pictures/photo.jpg -> ~/.ssh/id_rsa` выносила ключ на сервер под безобидным именем | Medium | `crates/sarca-sync/src/candidate.rs` | Ссылка следуется, только если её канонический путь остаётся внутри канонического корня биндинга; иначе запись пропускается |

## R4.2 Проверено и признано закрытым (без изменений)

- `strip_remote_root` уже отбивает `..`, `.`, `\` и `C:` в путях от сервера, так
  что классический zip-slip на `root.join(rel)` невозможен; поэтому фикс выше
  касается только симлинков.
- Граница IPC (`remote_ipc.rs`): `authorize_request` требует и `Origin`, и
  URL самого вебвью; `authorize_invoke` перепроверяет URL на каждый вызов, что
  закрывает переживший `disconnect` grant; `connect` / `get_url_history` не
  входят в `dispatch` вообще.
- `paths::validate_local_dir`: канонизация до сравнения, `starts_with` по
  компонентам (не по строке), запрет скрытых и служебных каталогов, запрет
  собственных каталогов приложения.
- CSP, capabilities, отсутствие `unsafe`, состояние зависимостей — без
  изменений с раунда 3.

## R4.3 Проверено чем

| Проверка | Команда | Результат |
| --- | --- | --- |
| Тесты sarca-sync | `cargo test -p sarca-sync --lib` | 66 passed (3 новых регрессионных теста на симлинки) |
| Тесты клиента | `cargo test -p sarca-client --lib` | 70 passed |
| Формат / клиппи | `cargo +nightly fmt -p sarca-sync --check`, `cargo clippy -p sarca-sync --all-targets -- -D warnings` | чисто |
| Линт сервера | `task lint` | чисто |

## R4.4 Что осталось проверить вручную

Список из R3.3 в силе целиком (подпись macOS/Windows, Isolation Pattern,
пентест API, ротация секретов). Дополнительно:

1. Поведение синка на Windows с junction points: `canonicalize` их резолвит, но
   отдельного теста под Windows на выход за корень нет.
2. Квоты на диск: лимит в 16 GiB на файл есть, суммарного лимита на биндинг нет.
