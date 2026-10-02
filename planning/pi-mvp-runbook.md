# Запуск Pi MVP

Локальный MVP Ariel реализован на Rust/Tokio/Axum/SQLite. Pi 0.87.1 выполняет
свой tool loop через native RPC внутри Docker. Панель владельца и независимый
HTML-клиент работают на разных loopback-портах. ACP для этого среза не нужен.

## Требования и первый запуск

Проверенный стенд: Windows, Rust 1.98, Node 24.13, Git 2.56, Docker Desktop
с Linux engine. Cargo.lock закрепляет зависимости: Axum 0.8.9,
rusqlite 0.40.2 с bundled SQLite, Tokio 1.53.1. Linux пока не квалифицирован.

Pi 0.87.1 должен быть установлен локально. При нестандартной установке задайте
`ARIEL_PI_PACKAGE` — каталог пакета `@earendil-works/pi-coding-agent`.
Это локальный executable MVP: бинарник использует exporter и Dockerfile из
этого checkout. Упаковка переносимой поставки — следующий этап.

Из корня репозитория:

```powershell
$env:LEKALO_BIN = (Get-Command lekalo).Source
node scripts/check-lekalo.mjs
cargo build --workspace --release --locked
.\target\release\ariel.exe init --import-pi-auth
.\target\release\ariel.exe runtime-build
.\target\release\ariel.exe doctor
.\target\release\ariel.exe start
```

`init --import-pi-auth` явно копирует имеющуюся авторизацию Pi. Общий каталог
Pi не монтируется в контейнер. Credential commands `!command` не исполняются;
неподдержанные конфигурации отклоняются. Каждый session profile получает
только credentials выбранного провайдера. Импорт или замена credentials
останавливает активные jobs; для нового profile revision нужна новая сессия.

Все модели и провайдеры изначально выключены. Откройте
`http://127.0.0.1:8787/`, введите `owner_token` из `.ariel/config.json`,
выберите провайдера, сохраните включение и включите точную модель.
Наличие credentials обозначено отдельно: это не проверка платного inference.

На вкладке «Тестовый клиент» скопируйте отдельный ключ клиента. Откройте
`http://127.0.0.1:8788/`, подключитесь, создайте папку, выберите модель,
начните сессию и отправьте работу. Например: «Создай README.md с описанием
простого HTTP-сервера». Файлы и события можно просмотреть на странице.
Смена модели или папки требует новой сессии; продолжение сохраняет старую
точную модель и историю Pi.

Ключи интерфейсов сохраняются только в sessionStorage браузера. Ключи
провайдеров не возвращаются в каталог/API настроек. Не публикуйте `.ariel`.

```powershell
.\target\release\ariel.exe status
.\target\release\ariel.exe stop
```

`--data DIR` задаёт отдельное состояние. `--bind` и `--test-bind` допускают
только явный `127.0.0.1:PORT`. Один data root обслуживает один daemon;
повторный запуск отклоняется системной блокировкой файла.

## Изоляция, вопросы и дополнительные пути

Pi работает как UID 1000 с read-only образом, без capabilities и host Docker
socket. Только собственный workspace, профиль сессии и явно разрешённые
пути попадают в bind mounts. Проверенный runtime receipt сохраняется в job.
Лимиты: 512 MiB RAM, 1 CPU, 128 PID; tmpfs 64 MiB. Workspace проверяется
каждые две секунды, при превышении 128 MiB задание останавливается.
Это наблюдаемый порог, а не жёсткая дисковая квота.

Панель владельца создаёт worktree из exact HEAD в собственном bare Git store.
Незакоммиченные изменения исходного checkout не копируются. Git links
настроены для `/workspace` внутри контейнера; Git-команды в этом worktree
выполняются внутри Pi runtime. Общий `.git` исходного репозитория не монтируется.
Symlink/reparse и файлы с несколькими hard links не принимаются при проверке.

Доверенное расширение предоставляет Pi инструменты:

- `ariel_ask_user`: input/select/confirm; обычный ответ может дать клиент.
- `ariel_request_path`: exact host path и read/write; решает только владелец.

Вопрос связан с job, attempt, session, generation, digest и expiry (5 минут).
Решение хранится атомарно; поздний ответ не возобновляет terminal job.
Path grant выдаётся на **один час для указанной сессии**. Владелец видит
исходный и canonical путь. Разрешение останавливает текущую попытку с
`interrupted`; только новый явный запрос запускает контейнер с новым mount.
Прозрачного повторения side effects нет. Read grant монтируется read-only.
Отзыв останавливает активные jobs workspace. При истёкшем grant удалите его
в панели перед следующим запуском.

После каждого задания контейнер удаляется с подтверждением отсутствия.
Следующий turn запускает новый контейнер и открывает сохранённый session
file Pi. После аварии daemon активные jobs становятся `interrupted` с
unknown outcome, принадлежащие этому data root контейнеры удаляются.
Автоматической повторной отправки нет. При неподтверждённой остановке сессия
blocked до успешного восстановления.

Docker bridge разрешает обычный egress. **Egress allowlist, hard disk quota,
protected internal paths и универсальный privacy scanner не реализованы.**
Процесс Pi видит собственный профиль с credentials; это не скрытый secret
broker. API редактирует известные credential literals, включая разделённые
text deltas, но не обещает защиту от кодирования или произвольной эксфильтрации.
Обязательное unsupported capability отклоняется до запуска; fallback на
неизолированный Pi на хосте отсутствует. Allowlist моделей относится к
admission Ariel и не является ограничением прав ключа на стороне провайдера.

## API для независимого клиента

Все `/v1/*` требуют `Authorization: Bearer TOKEN`. Owner и client tokens
различаются. Клиент видит только собственные workspace/session/job/question.
Допущены только точные origin двух локальных страниц; wildcard CORS отсутствует.

| Запрос | Поведение |
| --- | --- |
| `GET /health` | Версия, runtime, лимиты, URL тестовой страницы |
| `GET /v1/agents` | Pi, транспорт, capabilities и unsupported |
| `GET /v1/agents/pi/models` | Пересечение известных, configured и включённых точных моделей |
| `POST /v1/workspaces` | Клиент: `{mode:"folder"}`; owner также `worktree`, `source_path`, optional `source_revision` |
| `POST /v1/sessions` | `agent_id`, `provider_id`, `model_id`, `workspace_id`, optional `required_capabilities` |
| `POST /v1/jobs` | `session_id`, `input`, `idempotency_key`, optional `timeout_seconds` (30–600, default 300) |
| `GET /v1/jobs/{id}` | State, result/error и sandbox receipt |
| `GET /v1/jobs/{id}/events?after=N` | Durable SSE; `id`/`cursor` возрастают, replay не создаёт задания |
| `POST /v1/jobs/{id}/cancel` | `cancelling` → подтверждённая остановка → terminal |
| `GET /v1/questions` | Pending вопросы; canonical path доступен только owner |
| `POST /v1/questions/{id}/answer` | `request_digest` и `value` либо `confirmed` |
| `POST /v1/access-requests/{id}/decision` | Owner: `request_digest`, `decision:"allowed"|"denied"` |
| `POST /v1/workspaces/{id}/grants/{grant}/revoke` | Owner отзывает grant |
| `GET /v1/workspaces/{id}/files` и `/file?path=...` | Собственные артефакты; чтение до 1 MiB, traversal запрещён |

GET collections: `/v1/workspaces`, `/v1/sessions`, `/v1/jobs`. Owner endpoints:
`/v1/admin/catalog`, `/clients`, PUT `/providers/{id}`, `/models/{selection_id}`,
POST `/refresh`, `/import-pi`, `/shutdown`. `api_key` у provider write-only.
Body limit 64 KiB; job input до 32 KiB. Ошибка: `{error:{code:"..."}}`;
401 — нет авторизации, 403 — роль/origin/host, 404 — чужой/неизвестный объект,
409 — конфликт или отклонённый запрос.

Повтор того же idempotency key и того же payload возвращает прежний job,
включая после выключения модели. Другой payload с этим ключом — 409.
Одновременно выполняется одно задание на workspace и session.
State: queued/running/waiting_input/cancelling и terminal
succeeded/failed/cancelled/interrupted. `succeeded` требует `agent_settled`
и authoritative final assistant message; prompt ack/EOF/exit=0 недостаточны.
Unix timestamps выражены в секундах. SSE terminal и terminal job пишутся в
одной SQLite транзакции. HTML-клиент восстанавливает события после refresh
без повторного prompt; неизвестный ответ POST можно повторить с тем же ключом.

## Проверки и доказательства

```powershell
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
$env:ARIEL_BIN = (Resolve-Path .\target\release\ariel.exe).Path
node scripts/check-mvp.mjs
$env:LEKALO_BIN = (Get-Command lekalo).Source
node scripts/check-lekalo.mjs
```

`check-mvp` использует отдельный data root и Git fixtures в ignored
`test-results`, настоящий Pi/Docker и локальный deterministic model server.
Он не использует реальную авторизацию и не доказывает облачный inference.
Проверяет tools/историю, admission, ownership, origin/host, ошибки, SSE,
повторные POST, отмену shell descendants, worktree, вопросы, owner grant,
readonly/revoke и crash recovery. Его временный модельный endpoint слушает
случайный порт на host на время проверки, затем закрывается.

```powershell
# Явный живой тест: расходует лимит импортированной default-модели.
node scripts/check-live-pi.mjs
# Chromium: существующий agent-browser executable.
$env:AGENT_BROWSER_BIN = 'путь к agent-browser executable'
node scripts/check-browser.mjs
# Полная отправка задания кнопкой: также расходует лимит модели.
$env:ARIEL_BROWSER_LIVE = '1'
node scripts/check-browser.mjs
```

Проверки записывают redacted manifests в `test-results/{live-pi,mvp-conformance,
browser,lekalo}.json`. Cargo проверяет реализацию, Lekalo — semantic model.
Для Lekalo 0.6.3 девять проверок проходят; optional HLV отсутствует, поэтому
doctor `degraded`. Rust adapter/generator/native gate не подключены.
Полные GitHub issues автоматически не закрываются этим локальным MVP.
