---
title: rtok
tagline: CLI для сокращения токенов у ИИ-агентов для программирования — хуки, MCP-сервер и API-прокси в одном бинарнике на Rust.
repo: https://github.com/pyrlyn/rtok
homepage: https://github.com/pyrlyn/rtok
install: "curl --proto '=https' --tlsv1.2 -LsSf https://github.com/pyrlyn/rtok/releases/latest/download/rtok-installer.sh | sh"
install_alternatives:
  - 'ketch install pyrlyn/rtok'
  - 'brew install pyrlyn/tap/rtok'
  - 'npm i -g rtok-cli'
  - 'uv tool install rtok-cli'
version: "0.10.0"
accent: "#5CE1FF"
accent2: "#FF6B4A"
accentLight: "#0B7FA0"
featured: true
order: 1
lang: ru
---

<!-- Website copy for the pyrlyn project site. The sync-docs workflow copies this file to
pyrlyn/landing (main) as content/projects/rtok.md on every change to main and on every v*
tag; front matter follows CONTENT_CONTRACT.md in that repository.
Sources (checked 2026-09-27): README.md and the clap CLI in src/cli.rs; version from the latest
GitHub release (v0.10.0); accent from site/assets/css/custom.css (--rtok-accent). -->

## Обзор

`rtok` уменьшает контекст, который вынуждены нести ИИ-агенты для программирования. Это один бинарник
на Rust с тремя интерфейсами: хуки Claude Code, MCP-сервер и API-прокси. Каждое сокращение измеряется,
а укороченные данные остаются доступными по id.

Десять методов, один процесс, один журнал. Экономии, которой нет в виде строки в этом журнале, не
существует — включая экономию самого rtok.

## Возможности

- **Хуки для агентов.** `rtok agents install claude` прописывает хуки rtok и запись MCP
  в Claude Code, предварительно сохранив резервную копию файла настроек; `--dry-run` печатает дифф и ничего
  не трогает. Cursor, Codex, OpenCode, pi, Kimi, Copilot, Aider, Windsurf, Zed, VS Code и другие
  регистрируются так же.
- **Запускайте команды, не расплачиваясь за их вывод.** `rtok run -- <cmd>` сохраняет код выхода,
  архивирует сырой вывод и печатает компактную сводку с id, по которому его можно развернуть.
- **Без потерь by design.** Заархивированные данные возвращаются через `rtok expand <id>` — целиком, по диапазону
  строк или по совпадению с регулярным выражением. Хуки работают по принципу fail open: при любой ошибке агент получает свой ввод без изменений.
- **Граф кода через MCP.** `rtok graph index .` строит индекс символов, к которому агенты обращаются через
  `symbol`, `callers`, `impact`, `outline` и `explore` вместо цепочки «grep и прочитать».
- **Локальный API-прокси.** `rtok proxy` записывает расход, о котором сообщает провайдер, и может заменять старые
  результаты инструментов стабильными указателями на архив, сохраняя закешированный префикс.
- **Измерьте, прежде чем оставлять сокращение.** `rtok stats` читает транскрипты и расход через прокси;
  `rtok doctor` оценивает стоимость хуков, MCP-серверов и навыков, которые у вас уже работают.
- **Подключаемые методы.** `measure`, `cmd`, `read`, `archive`, `proxy`, `inject`, `guard`,
  `memory`, `graph` и `toon` — каждый можно отключить в едином файле конфигурации.
- **Наблюдаемость.** `rtok otel flush` экспортирует вызовы, логи и метрики в формате OTLP/HTTP JSON в
  Jaeger, Grafana, SigNoz или Maple. `rtok web` и `rtok tui` показывают те же данные локально.

## Установка

macOS (Apple silicon) и Linux x86-64, установка в `~/.cargo/bin`:

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/pyrlyn/rtok/releases/latest/download/rtok-installer.sh | sh
```

Через [ketch](https://github.com/pyrlyn/ketch), прямо из архивов GitHub Release:

```bash
ketch install pyrlyn/rtok
```

Homebrew, npm или PyPI (пакеты содержат тот же нативный бинарник):

```bash
brew install pyrlyn/tap/rtok
npm i -g rtok-cli
uv tool install rtok-cli
```

Из исходников:

```bash
git clone https://github.com/pyrlyn/rtok && cd rtok
mise install
mise exec -- cargo install --path .
rtok --version
```

## Примеры использования

Установите интеграцию с Claude Code: сначала предпросмотр, затем проверка:

```bash
rtok agents install claude --dry-run
rtok agents install claude
rtok doctor
```

Запустите команду через rtok и позже прочитайте заархивированный вывод:

```bash
rtok run -- cargo test
rtok expand 7f3a91 --lines 120-180
rtok expand 7f3a91 --grep "panicked" --context 3
```

Проиндексируйте дерево для графа кода и выведите список плагинов:

```bash
rtok graph index .
rtok plugins
rtok config set plugins.toon.enabled false
```

Запустите прокси и проведите измерения:

```bash
rtok proxy --mode passthrough
rtok stats --since 7d
rtok stats --save-baseline before-rtok
rtok stats --compare before-rtok
```

Посмотрите, откуда взялась каждая настройка:

```bash
rtok config show --sources
```

## Ссылки

- Репозиторий: <https://github.com/pyrlyn/rtok>
- Документация: <https://github.com/pyrlyn/rtok/tree/main/docs>
- Начало работы: <https://github.com/pyrlyn/rtok/blob/main/docs/getting-started.md>
- Релизы: <https://github.com/pyrlyn/rtok/releases>
- Список изменений: <https://github.com/pyrlyn/rtok/blob/main/CHANGELOG.md>
- Лицензия: на ваш выбор GNU GPLv3 или коммерческая лицензия (см. <https://github.com/pyrlyn/rtok#license>)
