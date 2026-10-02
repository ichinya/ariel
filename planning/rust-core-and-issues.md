# Rust-ядро и разбор backlog ARIEL

Дата проверки: 2026-09-30. Статус: принятое направление разработки и план
проверок; реализации службы и доказательств её runtime-поведения пока нет.

Последующее уточнение пользователя: первым runtime выбран Pi; работа с
файлами в выделенной папке/worktree входит в MVP. Актуальный порядок и
проверки находятся в [плане Pi MVP](pi-mvp.md). Ниже сохранён исходный обзор
backlog; его сведения о минимальной пустой модели — состояние до этого уточнения.

## Решение пользователя

Язык ядра — **Rust**, без промежуточной реализации на Go или Python.
ARIEL — самостоятельная служба с двусторонними агентскими сессиями,
событиями, вопросами пользователю и ожиданием разрешений. Внешние приложения
подключаются как клиенты; для локальной работы controller не обязателен.

| Часть | Выбор | Что ещё проверить |
| --- | --- | --- |
| Ядро, CLI, lifecycle | Rust | Сборка, shutdown и поставка на каждой заявленной ОС |
| Async I/O, процессы, сеть | Tokio | Bounded streams, cancellation, остановка descendants |
| ACP | Официальный Rust `agent-client-protocol` | Exact released SDK/protocol versions и полный цикл с mock-agent |
| Другие runtime | Отдельные structured RPC/CLI adapters | Их capabilities, auth и conformance без предположения ACP-совместимости |
| Jobs, attempts, вопросы, ответы, outbox | SQLite | Rust binding, транзакции, миграции, recovery и backup |
| Первый UI | Локальная web-панель | Scoped attachment, replay и typed actions; desktop/tray позже |

Подключение ACP использует структурированные сообщения. `stdout` протокола
и диагностический `stderr` обслуживаются отдельно. Terminal/PTY adapter —
отдельная возможность. Он не заменяет permission callbacks или correlation.

Документация официального SDK описывает клиентов, агентов, прокси, callbacks
и ordering: [Rust ACP SDK](https://agentclientprotocol.github.io/rust-sdk/).
Context7 также возвращает примеры из upstream `main`, включая feature-gated
v2. Перед кодом надо выбрать опубликованную версию, проверить её API и
совместимость с конкретным агентом. Не переносить v2-пример в v1-контракт
по совпадению названия crate.

Удаление Tokio `Child` по умолчанию не останавливает процесс. Supervisor
должен реализовать protocol cancel, bounded grace, platform-specific
termination и подтверждённый wait/drain. Остановка одного child не является
доказательством остановки всего дерева. Основание:
[Tokio process](https://docs.rs/tokio/latest/tokio/process/index.html).

## Что проверено сейчас

- Прочитан текущий список всех 78 issues репозитория: все OPEN.
- Получены 22 комментария в 18 issues; учтены уточнения durability,
  reconciliation, guards, session attachment и новых capabilities.
- В репозитории пока README, bootstrap-планирование и минимальная модель
  Lekalo. `Cargo.toml` и исполняемого Rust-ядра нет.
- Существующие `lekalo/project.yaml` и `lekalo/modules/app/module.yaml`
  загружаются в IR. Семантическая проверка: 0 errors, 0 warnings.
- Lekalo 0.4.0 создал `lekalo.lock` offline; `lock --check --offline` прошёл.
- После создания lock `doctor`: `degraded`, lock `fresh`; причина деградации
  — необязательная integration trace не передана. Это не ошибка модели и
  не подтверждение готовности будущей службы.
- Rust target/adapter/generator для ARIEL не настроен. Lekalo сейчас
  используется для модели, IR, валидации и воспроизводимого resolution.

Исходник backlog — [актуальные GitHub issues](https://github.com/ichinya/ariel/issues).
JSON в `planning/backlog/` содержит первоначальные задачи; более поздние
issues #69–#78 и комментарии нужно учитывать отдельно. Существующие native
blocking relationships не проверялись отдельным API. Текстовый `Depends on`
не считается доказательством созданного GitHub blocker.

## Расхождения с выбранным продуктом

| Issue | Найденное расхождение | Предлагаемая корректировка scope |
| --- | --- | --- |
| [#3](https://github.com/ichinya/ariel/issues/3) | Требует заново выбрать язык сравнением вариантов | Rust уже выбран. Сохранить измеряемый spike: mock lifecycle, дерево процессов, restart и пакет; reuse/license audit остаётся отдельной проверкой |
| [#2](https://github.com/ichinya/ariel/issues/2), [#4](https://github.com/ichinya/ariel/issues/4), [#8](https://github.com/ichinya/ariel/issues/8) | Нужен явный локальный путь без controller | Один execution core и контракт для локального UI, standalone API и connected mode |
| [#43](https://github.com/ichinya/ariel/issues/43) | Основной текст направляет вопросы через controller | Durable request/reply принадлежит ARIEL; локальный UI — полноценный способ ответа. Business question и technical permission остаются разными типами |
| [#44](https://github.com/ichinya/ariel/issues/44) | Standalone API сгруппирован в M4 | Минимальный локальный authenticated API нужен раннему UI; полный remote/client scope сохраняется отдельным этапом |
| [#74](https://github.com/ichinya/ariel/issues/74) | Viewer уже предусмотрен, но P0 read-only | Переиспользовать attachment/replay contract. После observer-фазы добавить разрешённые ответы и decisions через существующий broker |
| [#55](https://github.com/ichinya/ariel/issues/55) | Большая admin-панель находится в M6 | Не ждать quota/profile administration для первого экрана с сессией, вопросами и результатом |
| [#1](https://github.com/ichinya/ariel/issues/1) | Первый slice начинается с controller/local inference | Добавить ранний standalone agent slice. Inference-путь #29/#36 остаётся отдельным adapter, connected путь #45 следует за базовым lifecycle |
| [#69](https://github.com/ichinya/ariel/issues/69) | Публичная граница ещё требует работы | Новые контракты и fixtures нейтральны к приложениям. Bootstrap не запускать до согласования markers/lookup |

Это предложения для согласования backlog с новым решением. GitHub issues,
milestones, комментарии и зависимости в этой работе не изменялись.
Issue #3 не закрывается одним выбором Rust: измерений и reuse audit ещё нет.

## Карта всех issues

Группы ниже покрывают #1–#78 без пропусков. Номера — существующие owners,
а порядок групп не переписывает declared dependencies.

| Issues | Ответственность | Место в разработке |
| --- | --- | --- |
| #1 | Roadmap | Навигация, не runtime evidence |
| #2, #3, #4, #8, #9 | Архитектура, стек, публичный протокол, adapter, trust | Основа первого slice |
| #5, #7, #69 | Auth support, лицензия, публичные материалы | До обещаний поддержки, reuse и выпуска |
| #6 | Имя и публичное описание | Самостоятельно, не блокирует lifecycle |
| #10, #11, #15 | CLI/config, secrets и профили | Минимальный exact profile сначала; multi-account далее |
| #12, #16, #34 | Durable jobs/attempts, outbox, sessions/checkpoints | Основа restart/replay; native resume capability-specific |
| #13, #17, #18, #23 | Sandbox, permissions, supervisor, cancel | Обязательные гарантии выбранного workload |
| #14, #19, #20, #30, #45 | Client identities, transport, admission, discovery, connector | Локальные scopes сначала, connected node далее |
| #22, #78 | Conformance и startup benchmark | Synthetic fixtures сразу; измерения оптимизаций после baseline |
| #24, #25, #26, #27, #28 | Coding runtime adapters | Один подтверждённый transport сначала, остальные отдельно |
| #29, #36 | Inference и answer/analysis без Git | Независимый второй backend общего lifecycle |
| #21, #31, #32, #40, #41 | Projects, source delivery, worktree, Git auth | После базовой сессии, до coding writes |
| #33, #42, #50, #51, #61 | Artifacts, retention, commit/export/push | Результат сохраняется до cleanup; push отдельное разрешение |
| #43, #44, #74 | Attention, API, локальная session surface | Ранний локальный интерфейс вопрос/ответ |
| #52, #53, #63, #64 | API facade, tool ownership, MCP, SDK | После типизированных contracts, без второго tool/job store |
| #62, #68 | Consumer integration fixtures и составной E2E | Synthetic clients, workflow у потребителя |
| #37, #38, #46, #47, #48, #49, #58 | Model identity, quotas, assignment, telemetry | Exact identity/unknown semantics сразу; collectors и pools далее |
| #35, #54, #75 | Service installation, release, updater | После executable smoke; updater не условие первого выпуска |
| #55, #56, #57, #65 | Administration, tray, remote client, notifications | После минимального local session UI |
| #39, #59, #60, #66, #67 | Capacity, optional adapters/evaluation, browser/research | Optional capabilities, не блокируют MVP |
| #70, #71, #72, #73 | Workspace services/mirror, network observations, ingress | Отдельные owners поверх общего lifecycle |
| #76, #77 | Typed assessments, computer-use profiles | Optional; собственные capability и safety fixtures |

## Уточнения комментариев, которые нельзя потерять

- [#12](https://github.com/ichinya/ariel/issues/12): сохранить intent до
  dispatch, lease/fence, exact input/profile binding и operation-level
  invocation ref. UNKNOWN/404 не доказывает отсутствие эффекта. Reconcile
  читает исходный invocation, не выполняет его заново.
- [#16](https://github.com/ichinya/ariel/issues/16): ephemeral progress и
  durable terminal receipt раздельны; buffer overflow не теряет receipt.
  Lost ACK означает повтор доставки, а не повтор исполнения.
- [#17](https://github.com/ichinya/ariel/issues/17): deny любого mandatory
  guard блокирует действие независимо от порядка hooks; narrowed grants
  пересекаются. Изменённый target/args требует нового решения.
- [#18](https://github.com/ichinya/ariel/issues/18),
  [#23](https://github.com/ichinya/ariel/issues/23): stop новых effects,
  protocol cancel, grace и принудительная остановка своего дерева. PID
  без generation/start identity нельзя использовать для recovery kill.
- [#34](https://github.com/ichinya/ariel/issues/34),
  [#74](https://github.com/ichinya/ariel/issues/74): attachment lifecycle
  отличается от session/job lifecycle. Snapshot high-watermark и live
  replay должны иметь проверенную границу; detach не отменяет job.
- [#43](https://github.com/ichinya/ariel/issues/43): logical request ID,
  deadline, decision, delivery и actual effect раздельны. После restart
  одна pending карточка; concurrent allow/deny проходит single-decision CAS.
- [#42](https://github.com/ichinya/ariel/issues/42): сначала остановить
  writers и сохранить доступный результат/receipt, затем cleanup.
- [#48](https://github.com/ichinya/ariel/issues/48): exact model revision,
  approved aliases, deny override; разрешённый fallback одной attempt не
  меняет соседние profiles или child bindings.
- [#49](https://github.com/ichinya/ariel/issues/49),
  [#63](https://github.com/ichinya/ariel/issues/63): telemetry не является
  authoritative receipt; MCP composition сохраняет guards и ownership.

## Первый проверяемый slice

Предлагаемый порядок: контракты → Rust executable с fake runtime → SQLite
lifecycle/outbox/attention → supervisor/cancel → local API/view → ответы
и permissions → restart/reconnect → один разрешённый внешний runtime.

Минимальные semantic entities: Session, Job, Attempt, RuntimeActivation,
ProfileRevision, Event, AttentionRequest, PermissionDecision, Receipt.
Определять их в Lekalo вместе с публичными typed Rust contracts, не
создавать пустые «реализованные» сущности для всех 78 задач сразу.

| Проверка | Проверяемый результат | Owners |
| --- | --- | --- |
| Start + prompt | Handshake, session, явный input и correlated terminal response; exit=0 без результата — incomplete | #4, #8, #18, #22 |
| Question + reply | Одна persisted pending карточка, разрешённый one-shot reply, продолжение той же binding | #12, #34, #43, #74 |
| Permission | Allow/deny связан с exact request; expired, revoked и changed args дают 0 effects | #17, #43 |
| Cancel | Повтор безопасен, descendants остановлены, late reply не возобновляет работу | #18, #23 |
| Restart | Crash до/после decision и receipt не создаёт второй effect; UNKNOWN остаётся recovery_required | #12, #16, #34 |
| Reconnect | Snapshot/replay/live без дубля, detach не уничтожает job | #16, #34, #74 |
| Isolation | Два client scopes не видят чужие sessions, events и credentials | #9, #11, #13, #14 |
| Bounded I/O | Split/malformed frames, stdout/stderr flood и slow consumer не теряют durable receipt | #16, #18, #22 |

Эти критерии ещё не исполнялись. Windows/Linux process-tree guarantees,
реальный sandbox, published ACP SDK compatibility и SQLite crash tests
проверяются отдельным executable spike. Safe Rust и успешная сборка не
доказывают отсутствие lifecycle races. Первый UI не требует tray,
мобильного приложения, Git или подключения внешнего controller.

## Локальный рабочий цикл с Lekalo

Указать собранный бинарник через `LEKALO_BIN` либо установить CLI в PATH.
Из корня ARIEL в PowerShell:

```powershell
& $env:LEKALO_BIN --no-cache load --ir
& $env:LEKALO_BIN --no-cache validate
& $env:LEKALO_BIN --no-cache lock --check --offline
& $env:LEKALO_BIN --no-cache doctor
```

После добавления Rust workspace отдельные проверки — Cargo build/test с
lockfile и соответствующие lifecycle fixtures. Проверка пустой модели
Lekalo не подменяет compilation, runtime conformance или end-to-end UI.
