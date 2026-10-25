---
title: rtok
tagline: CLI, що скорочує витрату токенів ШІ-агентами для програмування — хуки, MCP-сервер і API-проксі в одному бінарнику на Rust.
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
lang: uk
---

<!-- Website copy for the pyrlyn project site. The sync-docs workflow copies this file to
pyrlyn/landing (main) as content/projects/rtok.md on every change to main and on every v*
tag; front matter follows CONTENT_CONTRACT.md in that repository.
Sources (checked 2026-09-27): README.md and the clap CLI in src/cli.rs; version from the latest
GitHub release (v0.10.0); accent from site/assets/css/custom.css (--rtok-accent). -->

## Огляд

`rtok` зменшує контекст, який мусять нести ШІ-агенти для програмування. Це один бінарник на Rust із трьома
інтерфейсами: хуки Claude Code, MCP-сервер і API-проксі. Кожне скорочення вимірюється, а
скорочені дані можна отримати назад за id.

Десять методів, один процес, один журнал обліку. Економії, якої немає рядком у цьому журналі, не існує —
зокрема й економії самого rtok.

## Можливості

- **Хуки для агентів програмування.** `rtok agents install claude` записує хуки rtok і запис MCP
  у Claude Code, попередньо створивши резервну копію файлу налаштувань; `--dry-run` виводить diff і нічого
  не змінює. Cursor, Codex, OpenCode, pi, Kimi, Copilot, Aider, Windsurf, Zed, VS Code та інші
  реєструються так само.
- **Запускайте команди, не платячи за їхній вивід.** `rtok run -- <cmd>` зберігає код виходу,
  архівує сирий вивід і друкує стислий підсумок з id, за яким його можна розгорнути.
- **Без втрат за задумом.** Заархівовані дані повертаються командою `rtok expand <id>` — цілком, за діапазоном
  рядків або за збігом регулярного виразу. Хуки працюють за принципом fail-open: за будь-якої помилки агент отримує свої вхідні дані без змін.
- **Граф коду через MCP.** `rtok graph index .` будує індекс символів, який агенти запитують як
  `symbol`, `callers`, `impact`, `outline` і `explore` замість ланцюжка «grep і читання».
- **Локальний API-проксі.** `rtok proxy` фіксує використання, про яке звітує провайдер, і може замінювати старі результати
  інструментів стабільними вказівниками на архів, зберігаючи кешований префікс.
- **Спершу виміряйте, потім залишайте скорочення.** `rtok stats` читає транскрипти й дані використання з проксі;
  `rtok doctor` оцінює вартість хуків, MCP-серверів і skills, які ви вже запускаєте.
- **Підключувані методи.** `measure`, `cmd`, `read`, `archive`, `proxy`, `inject`, `guard`,
  `memory`, `graph` і `toon` — кожен можна вимкнути в єдиному файлі конфігурації.
- **Спостережуваність.** `rtok otel flush` експортує виклики, логи й метрики як OTLP/HTTP JSON до
  Jaeger, Grafana, SigNoz або Maple. `rtok web` і `rtok tui` показують ті самі дані локально.

## Встановлення

macOS (Apple silicon) і Linux x86-64, встановлення в `~/.cargo/bin`:

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/pyrlyn/rtok/releases/latest/download/rtok-installer.sh | sh
```

За допомогою [ketch](https://github.com/pyrlyn/ketch), просто з архівів GitHub Release:

```bash
ketch install pyrlyn/rtok
```

Homebrew, npm або PyPI (пакети містять той самий нативний бінарник):

```bash
brew install pyrlyn/tap/rtok
npm i -g rtok-cli
uv tool install rtok-cli
```

Із вихідного коду:

```bash
git clone https://github.com/pyrlyn/rtok && cd rtok
mise install
mise exec -- cargo install --path .
rtok --version
```

## Приклади використання

Встановіть інтеграцію з Claude Code: спершу перегляньте зміни, потім перевірте:

```bash
rtok agents install claude --dry-run
rtok agents install claude
rtok doctor
```

Запустіть команду через rtok і згодом прочитайте заархівований вивід:

```bash
rtok run -- cargo test
rtok expand 7f3a91 --lines 120-180
rtok expand 7f3a91 --grep "panicked" --context 3
```

Проіндексуйте дерево для графа коду й перелічіть плагіни:

```bash
rtok graph index .
rtok plugins
rtok config set plugins.toon.enabled false
```

Запустіть проксі й виміряйте:

```bash
rtok proxy --mode passthrough
rtok stats --since 7d
rtok stats --save-baseline before-rtok
rtok stats --compare before-rtok
```

Подивіться, звідки взялося кожне налаштування:

```bash
rtok config show --sources
```

## Посилання

- Репозиторій: <https://github.com/pyrlyn/rtok>
- Документація: <https://github.com/pyrlyn/rtok/tree/main/docs>
- Початок роботи: <https://github.com/pyrlyn/rtok/blob/main/docs/getting-started.md>
- Релізи: <https://github.com/pyrlyn/rtok/releases>
- Журнал змін: <https://github.com/pyrlyn/rtok/blob/main/CHANGELOG.md>
- Ліцензія: на ваш вибір GNU GPLv3 або комерційна ліцензія (див. <https://github.com/pyrlyn/rtok#license>)
