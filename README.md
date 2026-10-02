# ARIEL / Ариель

**Agent Relay & Execution Layer** — независимое реле агентов и слой исполнения.

Русская игра названия: **«агент, реле и ель»**. `AR&EL` — возможная фирменная стилизация.

> Статус: локальный Pi MVP реализован. Дальнейшие сценарии ниже описывают целевой продукт.

## Запуск MVP

Из корня репозитория, при установленном Pi 0.87.1 и запущенном Docker Desktop Linux engine:

```powershell
cargo build --workspace --release --locked
.\target\release\ariel.exe init --import-pi-auth
.\target\release\ariel.exe runtime-build
.\target\release\ariel.exe start
```

Панель владельца: `http://127.0.0.1:8787/`; независимый тестовый клиент:
`http://127.0.0.1:8788/`. Ключ владельца и отдельные ключи клиентов находятся
в ignored `.ariel/config.json`. Включите провайдера и точную модель,
создайте рабочую папку/сессию и отправьте Pi работу.

Работают папки и отдельные Git worktree, события, вопросы, явные разрешения
на дополнительные пути, отмена, история Pi и восстановление после аварии.
Pi выполняется в контейнере; настройки и события сохраняются в SQLite.

[Инструкция, API, ограничения и воспроизводимые проверки](planning/pi-mvp-runbook.md).
Проверенный стенд — Windows; egress allowlist, жёсткая дисковая квота и
скрытие собственного credential profile от Pi пока не реализованы.

## Назначение

ARIEL устанавливается на компьютере или сервере и предоставляет приложениям доступ к разрешённым агентским runtime и моделям. Он подготавливает среду, исполняет назначенную задачу, возвращает события, результаты и артефакты.

Планируются два режима над одним execution core:

- **Connected node:** исходящее подключение к controller, получение задач и отправка результатов без обязательного входящего порта.
- **Standalone API:** собственный аутентифицированный API для приложений без зависимости от Orkora.

Orkora управляет экспертами, людьми, workflow, выбором модели и допустимостью fallback. ARIEL не дублирует эту оркестрацию. Задачер и другие приложения смогут использовать его независимо.

## Основные сценарии

- Два изолированных профиля одного Claude Code: GLM-compatible backend и Opus; отдельные auth/config/session scopes.
- Adapters Codex, Claude Code, Pi, OMP и OpenCode через подтверждённые ACP/native RPC/CLI interfaces.
- Запросы к существующему локальному inference endpoint.
- Answer, analysis и research без обязательного Git-репозитория.
- Кодовые задачи в отдельном workspace/worktree, локальный commit и optional controlled push либо возврат patch/bundle.
- Инструменты приложения с typed tool calls и continuation; разрешённые MCP tools.
- Передача вопросов, human approvals и artifact references controller-у.
- Discovery, quota/capacity snapshots и telemetry с явными unknown/stale значениями.

## Границы

Provider credentials по умолчанию остаются на узле. Возможность использовать подписку через CLI **не означает** право предоставлять её через произвольный third-party API: допустимые режимы проверяются отдельно для каждого provider/runtime.

Локальный CLI может обращаться к облачной модели. Местонахождение процесса не заменяет проверку фактического egress.

Неизвестная модель не получает рабочие или оценочные запросы автоматически. ARIEL не заменяет выбранную модель более слабой по собственной инициативе. Срочность не отменяет требования качества, локальности и безопасности.

Worktree и отдельная папка не считаются полноценным sandbox. Изоляция, секреты, permissions, cancellation и recovery входят в baseline MVP.

## План реализации

- [GitHub Issues](https://github.com/ichinya/ariel/issues)
- [Milestones](https://github.com/ichinya/ariel/milestones)
- [Первоначальные определения backlog](planning/backlog/)

Этапы: M0 — контракты; M1 — безопасный execution MVP; M2 — профили и adapters; M3 — Git/workspaces; M4 — API/tools/приложения; M5 — квоты и managed fallback; M6 — public beta и расширения.

Один issue описывает отдельную возможность или решение. В нём есть acceptance criteria, milestone и зависимости. Независимые ветки можно реализовывать параллельно; весь предыдущий milestone не является обязательной блокировкой.

Язык ядра — **Rust**, без промежуточной реализации на Go или Python. Сроки ещё не назначены.

## Техническая основа

- Rust — ядро, CLI и управление сессиями; Tokio — асинхронные процессы и сеть.
- Официальный Rust ACP SDK — ACP-подключения; отдельные structured RPC/CLI adapters — другие runtime.
- SQLite — durable jobs/attempts, вопросы, решения и журнал событий.
- Локальная web-панель — первый пользовательский интерфейс; desktop/tray позже.
- Lekalo — модель проекта, IR, валидация и lockfile. Native Rust adapter/generator для этого проекта пока не подключён.

Первый MVP: **Pi через native RPC**. В локальной панели пользователь включает нужных провайдеров и модели; отдельная тестовая страница получает каталог от Ariel, выбирает модель и отправляет Pi работу. Ariel выделяет папку или worktree, обеспечивает ограничения доступа, передаёт события, вопросы и результат. Отмена и reconnect входят в MVP.

[План Pi MVP, issues и критерии приёмки](planning/pi-mvp.md) фиксирует границы реализации. Semantic contracts поддерживаются в Lekalo; служба находится в `crates/ariel`, интерфейсы — в `web/admin` и `examples/pi-client`.

Dependency versions закреплены в Cargo.lock: Axum 0.8.9, Tokio 1.53.1 и rusqlite 0.40.2 с bundled SQLite. [Решение и разбор текущих issues](planning/rust-core-and-issues.md) отделяют направление от полного backlog. [AGENTS.md](AGENTS.md) задаёт рабочий цикл с Lekalo.

## О bootstrap плана

`.github/workflows/bootstrap-ariel-roadmap.yml` создаёт только планирование: milestones, labels, issues и declared blocking relationships. Он не собирает и не развёртывает продукт, не использует provider credentials, не удаляет пользовательские issues.

После создания рабочий источник истины — GitHub Issues. JSON-файлы являются исходным снимком планирования; редактирование JSON не переписывает существующие issues автоматически. Повторный bootstrap использует stable markers и не заменяет пользовательские описания.

## Лицензия

Лицензия и правила заимствования кода выбираются отдельной задачей до публикации релизов. Публичность репозитория не следует трактовать как уже выданную open-source лицензию на весь будущий код.
