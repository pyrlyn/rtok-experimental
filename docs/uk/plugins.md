---
lang: uk
---

# Плагіни

Кожен метод скорочення токенів у rtok — це плагін за одним трейтом. Плагіни — це вбудовані
модулі за Cargo features: жодного демона, жодних підпроцесів. Зовнішні плагіни `.wasm`
завантажуються лише у збірці з feature `wasm-host` (її немає в наборі за замовчуванням) і за
`[plugins.wasm] enabled = true`.

Стовпець **замінює** — це ціль специфікації, а не залежність: rtok заново реалізує
поведінку з нуля й ніколи не запускає, не лінкує й не читає названий інструмент.

| Плагін | Замінює (лише специфікація) | Інтерфейс |
|--------|----------------------|---------|
| [`measure`](../../src/plugins/measure/README.md) | rtk gain, headroom savings, lean-ctx gain | `stats`, `bench`, проксі |
| [`cmd`](../../src/plugins/cmd/README.md) | rtk hook, ctx_shell, bash_compress | PreToolUse(Bash) → `rtok run` |
| [`read`](../../src/plugins/read/README.md) | lean-ctx read/search/tree, read_cache | MCP `read` |
| [`json_tree`](../../src/plugins/json_tree/README.md) | — | proxy, MCP |
| [`archive`](../../src/plugins/archive/README.md) | CCR | proxy, MCP `expand`, `rtok archive rewrite` |
| [`proxy`](../../src/plugins/proxy/README.md) | caveman-proxy | `ANTHROPIC_BASE_URL`, `OPENAI_BASE_URL` |
| [`inject`](../../src/plugins/inject/README.md) | caveman/ponytail, lean-ctx | SessionStart, UserPromptSubmit |
| [`guard`](../../src/plugins/guard/README.md) | — | PreToolUse, `rtok guard check` |
| [`memory`](../../src/plugins/memory/README.md) | claude-mem | MCP, PreCompact |
| [`graph`](../../src/plugins/graph/README.md) | codebase-memory-mcp | MCP |
| [`toon`](../../src/plugins/toon/README.md) | — | proxy, MCP `expand` |

## Вимкнення плагінів

Під час виконання, у `~/.rtok/config.toml`:

```toml
[plugins.cmd]
enabled = false
```

Або під час збирання, щоб код узагалі не компілювався:

```bash
cargo build --no-default-features --features cmd,read
```

## Каталог кожного плагіна

| Файл | Містить |
|------|-------|
| `README.md` | що робить плагін і навіщо — на нього посилається таблиця вище |
| `AGENTS.md` | інваріанти й правила задач для того, хто з ним працює |
| `PLAN.md` | власний план збирання плагіна |

Як написати власний: [створення плагінів](plugin-authoring.md).

## Формат експорту графа

`rtok graph export` і MCP-інструмент `graph_export` видають один JSON-документ, `"schema": "rtok.graph.v1"`, описаний
файлом [`docs/schemas/rtok.graph.v1.schema.json`](../schemas/rtok.graph.v1.schema.json) (він генерується з типів Rust і
перевіряється тестом). Ключі верхнього рівня:

| Ключ | Містить |
|------|---------|
| `projects` | `id`, `name`, `root`, `origin`, `backend` (`tags`, `lsp` або `text`), `health` (`ok`, `stale`, `not indexed`, `missing`), `indexed_at` |
| `links` | `from`, `to`, `kind` (`manual` або `auto`), `reason`, `references` (посилання-виклики з `from` у `to`) |
| `nodes` | `id`, `project`, `kind`, `name`, `path` (відносно проєкту), `line`; порожній на рівні `overview` |
| `edges` | `from`, `to` (id вузлів), `kind` |
| `meta` | `scope`, `level` (`overview`, `symbols` або `focus`), `focus`, `depth`, `exported_at`, `rtok_version`, `redacted`, `partial`, `notes` |

Абсолютні шляхи, домашній каталог і ім'я користувача приховуються, якщо не вказано `--no-redact` (`graph_export`
приховує завжди); вихідний текст не включається ніколи, імена файлів і символів включаються. Виклик, чия назва має
більше восьми визначень, ребра не малює. `--from FILE` показує збережений експорт, не
читаючи й не змінюючи реєстр та індекс.

### Картинки

`rtok graph export --format svg` малює той самий експорт картинкою, а `--format png` растеризує цю картинку (потрібен
`-o FILE`, `--scale` від 1 до 4 — кратність розміру SVG). Картинка залежить лише від експорту, тож `--from FILE`
малює збережений експорт точно так, як живу область. Проєкти — панелі, символи — крапки на спіралі (найзв'язаніші в
центрі; коло — функція, квадрат — тип, ромб — модуль), суцільна стрілка — виклик усередині проєкту, пунктирна —
виклик між проєктами, товста лінія — зв'язок проєктів. Малюється не більше 200 вузлів; підвал каже, скільки
приховано, а JSON завжди містить усі. У підвалі також проєкти з бекендом і `indexed_at`, область, версія rtok, час
експорту та, якщо проєкт не зміг відповісти, позначка PARTIAL. Для PNG на машині потрібен шрифт; без нього команда
так і скаже, а не намалює картинку без тексту. MCP `graph_export` лишається JSON.

### На сторінці Graph

На сторінці Graph є кнопка Export і вибір файлу «open an export». Кнопка відкриває панель, де сказано, що до
файлу потрапляють імена файлів і символів (вихідний текст ніколи) і що домашній каталог, ім'я користувача та
абсолютні шляхи замінено; далі можна вибрати огляд, граф символів відкритого проєкту і, якщо вибрано функцію,
підграф навколо неї, у форматі JSON, SVG або PNG. Файл пише сервер тією самою функцією, що викликає `rtok graph
export`, тому JSON збігається з виводом CLI байт у байт, окрім `exported_at`; сторінка лише зберігає його.
Картинки живого кадру в цьому меню немає. Вибір файлу надсилає збережений JSON-експорт серверу текстом; сервер
перевіряє його як `export::read` і повертає: сторінка показує його лише для читання під написом
«viewing export from FILE» і нічого не пише в реєстр та індекс. У `rtok tui` ті самі дії з'являться в T329.41.
