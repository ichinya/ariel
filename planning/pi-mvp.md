# MVP: Ariel → Pi → рабочая папка

План: 2026-09-30. Обновление: 2026-10-01 — локальный исполняемый MVP готов.
Rust/Tokio/Axum/rusqlite, два экрана, Pi RPC, Docker, folder/worktree,
вопросы/grants/cancel/replay/recovery реализованы. Подробности, API и ограничения:
[pi-mvp-runbook.md](pi-mvp-runbook.md). Исторические preflight-наблюдения ниже
сохраняются отдельно от новых runtime-проверок; полные issues не закрыты.

## Пользовательский результат

1. Пользователь запускает Ariel и открывает локальные настройки.
2. Ariel показывает провайдеров и модели, известные установленному Pi,
   отдельно обозначая доступность credentials и состояние runtime.
3. Пользователь включает только нужных провайдеров и точные модели.
4. Отдельная маленькая тестовая HTML-страница подключается к публичному
   API Ariel, получает агента `pi` и разрешённые доступные модели.
5. Пользователь выбирает модель, workspace и отправляет работу. Pi ведёт
   собственный агентский/tool loop; Ariel передаёт вход, события и вопросы.
6. Ariel подготавливает отдельную папку либо worktree. Pi и его дочерние
   процессы не получают доступ к другим host-папкам без точного разрешения.
7. Страница показывает текст, tool progress, вопрос, итог и отмену. Refresh
   страницы не создаёт второй запрос. Изменённые файлы остаются в workspace.

Это coding harness, а не только диалог. Простой `2+2` — transport smoke;
приёмка MVP также требует задания, изменяющего файл внутри workspace.

## Принятые решения

| Решение | Граница |
| --- | --- |
| Rust + Tokio + SQLite | Ariel владеет API, configuration, admission, lifecycle, store и supervisor |
| Pi native RPC / JSONL | Первый runtime; ACP SDK нужен последующим ACP adapters и не блокирует Pi |
| Pi 0.87.1 | Проверенная локальная baseline, не утверждение «последняя версия»; runtime image pin до live запуска |
| Pi owns tools | Ariel не реализует ещё один planner/tool loop; raw RPC не отдаётся браузеру |
| Один активный job на Pi session | Session events не имеют request ID; сериализация привязывает их к одной attempt |
| Отдельный Pi process/container на session | Общая изменяемая выбранная модель между клиентами запрещена |
| Local owner UI + отдельный test client | Управление конфигурацией и исполнение имеют разные grants |
| Docker Linux container на первом стенде | Весь Pi внутри; только утверждённые workspace mounts, свой state, без host socket |
| Lekalo для semantic contracts | Модель, IR, graph, context и negative probes; Cargo/runtime tests обязательны отдельно |

Первые целевые среды: Rust host на Windows с Docker Desktop Linux engine;
Linux host — после тех же conformance tests. Наличие Docker не доказывает
готовность изоляции. При неподдержанной обязательной границе запуск blocked,
без автоматического перехода к Pi с правами пользователя хоста.

## Каталог и переключатели

Разделить `known`, `configured/available`, `enabled`, `client-granted` и
`ready`. Включение provider не включает все его текущие и будущие модели.
Каждая модель разрешается exact парой `(provider_id, model_id)` для `pi`.
Discovery, auth presence и успешная проверка paid inference — разные факты.

Owner UI получает полный известный каталог и причины недоступности.
Client API возвращает пересечение включённого provider/model, доступного
профиля, client grant и требуемых capabilities. Ariel повторяет admission
перед запуском: скрытая в UI модель должна отклоняться и при прямом HTTP.
Сохранение переключателей атомарно; restart сохраняет выбор. Отключение
запрещает новые turns, активный turn следует явной stop/finish policy;
credential/grant revoke немедленно блокирует новые эффекты.

В Pi 0.87.1 RPC `get_available_models` не заменяет полный каталог настроек.
Для known catalog подходит read-only exporter установленного Pi SDK:
`ModelRuntime.getProviders()` / `getModels()`, offline, без auth discovery
из чужого HOME. Это маленький компонент Pi adapter, выполняемый имеющимся
Node runtime; ядро Ariel остаётся Rust. Exporter отдаёт только allowlisted
metadata, исключая headers, credentials, config commands и приватные URLs.
Версия exporter/SDK связана с runtime profile. Глобальное обновление Pi не
должно незаметно изменить закреплённую сессию.

Credentials первоначально настраиваются локально через поддержанный Pi
auth/API-key путь в отдельном profile directory. Browser получает auth
status, но не содержимое секрета. Общий host `~/.pi/agent`, все env keys и
наследуемые shell credential commands не передаются автоматически.
OAuth UI всех providers и универсальный secret manager не нужны первому
slice; один реально разрешённый auth path должен работать end-to-end.

## Workspace и разрешения

`cwd` и worktree не ограничивают файловые права Pi. Официальная
[security documentation](https://github.com/earendil-works/pi/blob/v0.87.1/packages/coding-agent/docs/security.md)
прямо описывает эту границу. Первый backend запускает весь Pi в контейнере,
а не только shell tool. Runtime-файлы образа и собственные temp/state roots
доступны процессу; речь о запрете произвольного доступа к файлам хоста.

- Папка создаётся в managed root с owner/client/session/attempt identity.
- Git-вариант использует exact source revision и отдельный worktree.
  Для writable Git metadata требуется собственная managed копия Git store;
  нельзя монтировать общий `.git` пользовательского checkout ради работы
  ссылки `gitdir`. Альтернатива первого прототипа — bounded source snapshot
  без заявления, что это полноценный writable worktree.
- Mount plan имеет canonical paths, read/write mode, generation и digest.
  Нельзя монтировать host root, Docker socket, общий profile/home или
  соседние checkout по ошибке относительного пути.
- Backend проверяет non-root, read-only image, capabilities, resource
  limits и lifetime контейнера. Эти меры не заменяют проверку mounts.
  См. [Docker bind mounts](https://docs.docker.com/engine/storage/bind-mounts/).
- Доступ к provider требует egress. Обычный Docker bridge не является
  provider allowlist: profile должен объявлять реальный network режим.
  Для требуемого запрета host/LAN/metadata нужен проверенный network gate;
  без него соответствующий strict workload не допускается.
- Дополнительный путь запрашивается типизированно: exact canonical path,
  access mode, job/attempt, workspace generation, expiry и request digest.
  Решение владельца хранится до ответа; timeout/deny не расширяют mounts.
- Расширение запускает проверенный новый mount plan/generation после
  остановки старых writers. Совместимое Pi resume проверяется отдельно;
  если оно недоступно — явная новая attempt с сохранённым результатом,
  без обещания прозрачного повторения side effects.

Доступ за границы проверяется реальными read/write probes: absolute path,
`..`, symlink/reparse, shell descendant, sibling workspace, Git metadata,
и после revoke. Read-only grant не разрешает write. Успешный тест внутри
папки не является достаточной проверкой изоляции.

## Pi adapter

Upstream: [Pi 0.87.1 RPC](https://github.com/earendil-works/pi/blob/v0.87.1/packages/coding-agent/docs/rpc.md)
и [RPC commands](https://github.com/earendil-works/pi/blob/v0.87.1/packages/coding-agent/docs/rpc-commands.md).
Подписка на stdout/stderr устанавливается до отправки запроса.

| Действие | Протокол / правило |
| --- | --- |
| Discovery | `get_available_models`; data проецируется, raw config не экспортируется |
| Выбор | `set_model {provider, modelId}`, затем `get_state` и сравнение exact identity |
| Работа | `prompt {id, message}` после persist/admission; success response означает приём |
| Ответ | `message_update` с `text_delta`; tools/events отдельно; hidden reasoning не публиковать |
| Итог | `agent_settled` + authoritative final message/stop reason; error/aborted не превращать в succeeded |
| Отмена | `abort` с bounded wait; затем owned container stop/kill и подтверждение отсутствия writers |
| Вопрос | Поддержанный `extension_ui_request` → typed AttentionRequest → matching response; текст не выдаёт grant |
| Продолжение | Новый serial job в той же живой session с прежним profile binding |
| Restart | Persisted history/receipts; активный job interrupted/recovery_required, если resume не доказан |

В Pi v0.87.1 `agent_end` закрывает low-level run; retries, compaction и
follow-ups могут продолжиться. Единственный `agent_end`, EOF или exit=0
не завершают Ariel job. JSONL разбивается по LF, принимает CRLF, сохраняет
Unicode separators внутри JSON. Buffers и replay ограничены; terminal
receipt имеет durable delivery state.

Project extensions/skills/configuration не наследуются автоматически.
Для initial profile используются явные resource flags и только доверенная
версия bridge-extension, если она нужна для вопросов. `--no-tools` применён
только в metadata probe; рабочий MVP включает Pi tools внутри sandbox.
Pi `enabledModels`/`--models` — startup/cycling scope, не ACL Ariel.

## API и два экрана

Этот API реализован; дополнительные owner/workspace/question endpoints
и форматы описаны в runbook:

| Surface | Минимум |
| --- | --- |
| Owner settings | Known providers/models, enabled toggles, auth/runtime/isolation status, save, refresh |
| `GET /v1/agents` | `pi` с version, transport, readiness и capabilities |
| `GET /v1/agents/pi/models` | Только exact разрешённые доступные entries и catalog revision |
| `POST /v1/sessions` | Exact model/profile + approved workspace ref; admission до spawn |
| `POST /v1/jobs` | Session, prompt, idempotency key, deadline; durable job ID |
| `GET /v1/jobs/{id}` | State и terminal receipt |
| `GET /v1/jobs/{id}/events` | SSE + cursor/replay; bounded event DTO |
| `POST /v1/jobs/{id}/cancel` | Idempotent cancel; delivery и подтверждённая остановка раздельны |
| `POST /v1/access-requests/{id}/decision` | Только owner grant, exact digest/generation; enum allowed/denied |
| Test HTML client | URL Ariel, client credential, agent/model/workspace selectors, prompt, stream, cancel, next turn |

Test client находится в `examples/pi-client/`, не читает SQLite/Pi files
и не запускает Pi сам. Его можно обслуживать отдельным локальным HTTP
server. Default API bind — loopback; точный test origin в allowlist,
без wildcard CORS или разрешения `file://`/null origin. Owner и client
credential различаются; provider secrets отсутствуют в обеих проекциях.
Уведомление о запросе дополнительного пути ведёт в owner UI.

## Owners в существующем backlog

Новые issues для этого среза не нужны. Комментарии отмечают ранний MVP
subset; полные acceptance criteria и milestones автоматически не закрываются.

Опубликованы и проверены повторным чтением 13 комментариев: [сводный маршрут
в #1](https://github.com/ichinya/ariel/issues/1#issuecomment-5905555779),
[Pi adapter в #26](https://github.com/ichinya/ariel/issues/26#issuecomment-5905557794),
[catalog в #30](https://github.com/ichinya/ariel/issues/30#issuecomment-5905558223).
Полные тексты — [pi-mvp-comments.json](pi-mvp-comments.json),
все ссылки и результаты проверок — [pi-mvp-evidence.json](pi-mvp-evidence.json).

| Issues | Роль в MVP |
| --- | --- |
| #1, #3 | Pi-first маршрут, Rust decision и измеряемый spike |
| #4, #8 | Typed request/event/receipt и runtime adapter boundary |
| #10, #11, #15 | Старт Ariel, отдельный Pi profile, configuration и credentials |
| #30, #37, #55 | Known/available/enabled catalog, owner switches и client projection |
| #26 | Pi RPC, точная модель, streaming, completion, continuation |
| #12, #16, #18, #23, #34 | Store, outbox, supervisor, cancel, session lifetime |
| #13, #17, #21, #40, #43 | Реальная FS boundary, path grants, folder/worktree, вопросы |
| #44, #74 | Минимальный standalone API и независимая HTML test surface |
| #22 | Fixture suite и реальный MVP acceptance |

#5 остаётся владельцем разрешённого provider auth; #9 — security invariants.
Remote pairing/controller, остальные agents, SDK facade, quotas, tray,
autoupdate, push и глобальная оркестрация идут после этого среза.

## Последовательность реализации

| Шаг | Артефакты / место | Gate |
| --- | --- | --- |
| P0 — модель и preflight | `lekalo/modules/app/*`, диагностические scripts, этот план | Canonical IR, graph/context, negative reference; Pi metadata RPC без prompts |
| P1 — Rust bootstrap и profile/catalog | `Cargo.toml`, `crates/ariel-core/src/{catalog,profiles,store}`, `crates/ariel-cli` | `start/status/doctor`; disabled по умолчанию; config restart; secret-free DTO |
| P2 — workspace и sandbox | core `workspace`, `sandbox`, pinned Pi image | Read/write внутри; снаружи denied; known network/resource guarantees; unsupported до spawn |
| P3 — Pi lifecycle | core `adapters/pi`, `supervisor`, `jobs`, `events` | Fake JSONL + installed Pi; exact selection; file-edit task; error/cancel/EOF; no duplicate dispatch |
| P4 — API и UI | core `api`; `web/admin`; `examples/pi-client` | Owner toggles → client catalog → submit/stream/cancel/next turn; unauthorized direct calls rejected |
| P5 — grants и recovery | core `permissions`, `attention`; integration fixtures | Exact external-path decision, expiry/revoke, UI reconnect, daemon crash, stopped descendants |
| P6 — acceptance | `tests/pi-mvp/`, runbook и evidence manifest | Полный пользовательский путь на выбранном профиле; Cargo + Lekalo + browser/runtime checks |

P1 выбирает и закрепляет версии HTTP/SQLite crates и Cargo.lock; Node/Pi
поставляются pinned в runtime image. P2 реализуется до выполнения model-
generated команд. P3 можно разрабатывать на fake process параллельно P2,
но это не разрешает live coding вне verified boundary.

Для этого прохода статус: **P0 выполнен; P1–P6 запланированы**.

## Приёмка MVP

| ID | Проверка | Успех / отрицательный контроль |
| --- | --- | --- |
| MVP-01 | Owner выбирает provider/model | Выбор переживает restart; новый model ID остаётся disabled |
| MVP-02 | Test client читает каталог | Виден `pi` и разрешённые ready models; forged disabled selection rejected до Pi |
| MVP-03 | Отправка работы | Pi использует exact model и меняет fixture-файл в workspace; final result доступен |
| MVP-04 | Folder/worktree | Исходный dirty checkout сохранён; соседний workspace и общий Git store недоступны |
| MVP-05 | Запрет выхода | Read/write через paths, links и shell descendants blocked вне grants |
| MVP-06 | Явное расширение | Только утверждённый путь/mode в новой generation; deny/expiry/revoke ничего не расширяют |
| MVP-07 | Отмена | UI cancel завершает работу и writers; повтор и late events не создают новую работу |
| MVP-08 | Сессия и reconnect | Следующий запрос получает нужный контекст; refresh не повторяет prompt; replay не теряет итог |
| MVP-09 | Ошибки и restart | Unknown outcome сохраняется; error/abort/EOF не маркируются succeeded; исходные receipts retained |
| MVP-10 | Credentials и клиентские scopes | Secrets не попадают в HTTP/events/browser; два клиента не видят чужой state |
| MVP-11 | Lekalo как рабочий инструмент | Модель меняется вместе с контрактом; checks, graph/context и negative test воспроизводимы |

## Исторический preflight 2026-09-30

`scripts/probe-pi-rpc.mjs` запустил установленный Pi 0.87.1 в отдельном
temporary config, offline, без real credentials и prompts. Подтверждены
catalog, exact synthetic selection, get_state, unknown-model rejection,
idle abort и EOF shutdown. Metadata SDK дал 42 known providers и 1497 known
models; RPC показал 2 искусственно настроенные модели. Это snapshot
установленного каталога, а не число доступных пользователю paid моделей.
Docker Desktop Linux engine ответил на version/info; контейнерная изоляция
и active process-tree cancellation пока не проверены.

`scripts/check-lekalo.mjs`: 9 checks PASS на модели из 58 definitions.
Проверены deterministic IR, semantic validation, fresh lock, graph,
inspect, context capsule, отказ unresolved effect, отсутствие cache writes
в fixture и обязательные doctor checks. `doctor` остаётся `degraded` из-за
optional HLV trace. Context честно показывает `detected-effects-absent`,
`error-contracts-unrepresentable`, `no-policies`: настоящая enforcement-
логика ещё не реализована. Условные runtime policies не подменены
безусловным `allow` в модели.

При заполнении модели Lekalo отклонил `required:false`; optional задан через
type wrapper. JSON ошибки CLI может идти в stderr — checker учитывает exit
code и оба канала. Ни один из этих случаев не требует изменения Lekalo.

После обновления project description `lock --check` обнаружил stale lock.
Применён exact plan после `update --dry-run --offline`; после этого все
9 checks прошли. Финальная проверка выполнена CLI 0.5.0; digest binary,
plan ID и lock digests сохранены в evidence. Ранняя проверка сборки CLI
0.4.0 и финальная проверка текущего binary — разные наблюдения.

Команды из корня проекта; `LEKALO_BIN` указывает на установленный binary:

```powershell
node scripts/check-lekalo.mjs
node scripts/probe-pi-rpc.mjs <installed-pi-package-directory>
```

Эти исторические проверки доказывают только metadata/preflight.

## Реализация и runtime-проверки 2026-10-01

Первые executable crates объединены в `crates/ariel`; Node exporter является
небольшим компонентом Pi adapter. Dependencies закреплены Cargo.lock.
Два живых последовательных turn через существующую авторизацию Pi создали
и прочитали `mvp-smoke.txt`; точный marker и terminal SSE проверены.

`scripts/check-mvp.mjs` выполняет отдельный deterministic model fixture с
настоящим Pi 0.87.1 и Docker. Подтверждены exact admission, client scope,
Host/Origin, файл и история, durable terminal/cursor, model error,
idempotency/serialization, cancellation shell descendants, owned worktree,
неизменность source checkout, owner-only canonical path grant,
readonly/revoke, typed clarification и crash recovery без redispatch.
API редактирует известные credential literals, в том числе text deltas,
разделённые между сообщениями. Универсальный privacy guard не заявляется.

`scripts/check-browser.mjs` проверяет два экрана в Chromium; Cargo tests
и Clippy проверяют Rust. `scripts/check-lekalo.mjs` проходит девять проверок
на Lekalo 0.6.3 и модели из 80 definitions. Lock fresh; preview update не
требует изменения dependencies. Optional HLV отсутствует; Rust generator,
native gate и adapter не настроены. Модель не заменяет runtime evidence.

Новые redacted manifests — `test-results/live-pi.json`,
`mvp-conformance.json`, `browser.json`, `lekalo.json`; каталог ignored.
Старый `pi-mvp-evidence.json` относится к planning/preflight и не переписан.

Grant имеет явно показанный owner UI срок один час и **session scope**;
origin job/attempt и workspace generation записаны. Текущая попытка
останавливается, следующий явный turn применяет mount. Worktree links
ориентированы на контейнерный `/workspace` из-за Git 2.39 в pinned image.
Общая пользовательская `.git` не предоставляется контейнеру.

Остаются за пределами квалифицированного локального среза: Linux,
egress allowlist, hard disk quota, protected internal credentials,
универсальная privacy policy, переносимая поставка и все adapters кроме Pi.
Точное время истечения questions/grants и полный набор adversarial probes
из широких issue acceptance criteria требуют дальнейшей квалификации.

Обновление реализации опубликовано и проверено чтением в
[roadmap #1](https://github.com/ichinya/ariel/issues/1#issuecomment-5932267334).
Код остаётся локальным незакоммиченным результатом; commit/PR этим этапом
не создавались. Итоговый manifest локально: `test-results/acceptance.json`.
