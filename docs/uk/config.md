---
lang: uk
---

# Конфігурація

Один файл, `~/.rtok/config.toml`, містить усі налаштування rtok. **Кожен прапорець CLI — це ключ
конфігурації**, тож усе, що можна передати в командному рядку, можна й зробити постійним, а
`rtok config show --sources` завжди скаже, звідки взялося значення.

Статус: T12.1–T12.2 виконано — кожна таблиця нижче є типізованим розділом у `crates/rtok-config/src/lib.rs` (невідомий
ключ = помилка), `crates/rtok-config/default.toml` вбудовано в бінарник, і `rtok config init` записує його дослівно,
`core.inject_budget_tokens` перенесено до `plugins.inject.budget_tokens`, а нашарування
(user < project < env < flags, на основі `figment`) разом із `config show [--sources] [--json]` і
`config get <key>` уже працюють. `config set`/`validate` і тест покриття прапорців з'являться в T12.3–T12.4.

## Пріоритет

Від найнижчого до найвищого. Пізніші рівні перевизначають попередні ключ за ключем.

1. **Вбудовані типові значення** — значення з довідкового файлу нижче (`crates/rtok-config/default.toml`,
   вбудованого в бінарник).
2. **Файл користувача** — `~/.rtok/config.toml` (каталог можна перевизначити через `RTOK_HOME`; файл —
   через `RTOK_CONFIG=<path>` або `--config <path>`).
3. **Файл проєкту** — `<git root>/.rtok.toml`, якщо він є. Схема конфігурації з обмеженнями
   довіри нижче; наприклад, дозволені `[plugins.read] default_mode` і `[plugins.inject] budget_tokens`.
4. **Файли `.env`** — рядки `RTOK_*` (ті самі назви, що й на рівні змінних середовища) з найближчого
   `.env` у робочому каталозі або вище, потім із `~/.rtok/.env`; дозволені ключі проєкту перемагають.
   Обмеження довіри такі самі, як для TOML проєкту. Домашній файл є довіреним;
   відхилений ключ проєкту не приховує значення з домашнього файлу.
   Вони лише розбираються, але ніколи не експортуються: команди, запущені через `rtok run`, їх не успадковують, а інші
   ключі в `.env` проєкту ігноруються. Про некоректний файл повідомляється в stderr, і він пропускається.
5. **Змінні середовища** — `RTOK_<SECTION>_<KEY>` у верхньому snake case, наприклад `RTOK_PROXY_PORT=8791`,
   `RTOK_PLUGINS_CMD_REWRITE=false`, `RTOK_STATS_SINCE=7d`. Списки розділяються комами.
   Наявні короткі назви лишаються псевдонімами: `RTOK_UPSTREAM`, `RTOK_OPENAI_UPSTREAM`.
6. **Прапорці командного рядка** — `rtok proxy --port 8791`.

Файли проєкту не можуть налаштовувати розділи `core`, `otel`, `proxy`, `mcp`, `web`
(і застарілий `dashboard`), `log`, `demon`, `agents`, `setup`, `doctor`, `stats`, `report`,
`bench`, `filter`, `worktree`, а також плагіни `memory`, `compress`, `graph`, `docs`, `guard`,
`wasm`. Заборонені й ключі `plugins.cmd.shell`, `plugins.cmd.rules`, `plugins.cmd.rules_dir`,
`plugins.read.allow_paths`, `plugins.inject.modes_dir`, `tasks.gitlab.url`.
Вони керують мережевими адресами, обліковими даними, виконанням команд і доступом до файлів.
Обмеження діють і для застарілих псевдонімів. Попередження містить файл і ключ без значення;
`config show --sources` позначає збережене безпечне значення як `ignored (project)`,
якщо його не перевизначив вищий рівень. Задавайте ці параметри в конфігурації користувача,
`~/.rtok/.env`, середовищі процесу або явно обраному файлі `--config`.
Вибір `--config` означає явну довіру файлу, зокрема всередині проєкту.

Правила:

- Позиційні аргументи окремого виклику (`hook <event>`, `expand <id>`, `run -- <cmd>`,
  `setup <host>`, `memory import <file>`, `--save-baseline <name>`) не є налаштуваннями й не мають ключа. Усе
  інше має.
- Прапорець `--foo-bar` підкоманди `baz` ↔ ключ `baz.foo_bar`. Налаштування плагінів живуть у
  `plugins.<id>.<key>`.
- Невідомі ключі — це помилка в `rtok config validate` і попередження (у stderr, один раз) деінде.
  Хуки ніколи не падають через проблеми з конфігурацією: вони записують це в лог і використовують типові значення (fail open).
- Шляхи приймають `~`. Відносні шляхи відраховуються від файлу, у якому вони записані.
- Тривалості: `30d`, `12h`, `15m`. Розміри: прості цілі числа в байтах або токенах, як зазначено в назві.

## `rtok config`

| Команда | Що робить |
|---------|------|
| `rtok config show [--sources] [--json]` | фактична конфігурація після всіх рівнів; `--sources` позначає кожен ключ як `default / user / project / env / flag` |
| `rtok config init [--force]` | записує довідковий файл (нижче) у `~/.rtok/config.toml`; без `--force` ніколи не перезаписує |
| `rtok config validate [path]` | розбирає файл, відхиляє невідомі ключі й значення поза діапазоном із номерами рядків; у разі помилки код виходу 1 |
| `rtok config path` | виводить визначений шлях до файлу користувача (і до файлу проєкту, якщо є) |
| `rtok config get <key>` | виводить одне фактичне значення, наприклад `rtok config get proxy.port` |
| `rtok config set <key> <value>` | редагує файл користувача на місці, зберігаючи коментарі (використовує `toml_edit`) |

Облікові дані ніколи не виводяться: `otel.headers` (містить ключі прийому OTLP) і `mcp.token` (bearer-токен
`rtok mcp --http`) після задання показуються як `<redacted>` у `show`, `get`, `set` і `rtok report` — їхнє
джерело все одно показується. Щоб побачити значення, прочитайте сам файл.

## Довідковий файл

Це дослівний `crates/rtok-config/default.toml`. Кожне показане значення — типове; свіжий
`rtok config init` записує саме це. Логування налаштовується лише в `[log]` (D26). `core.log_file` / `log_level` / `log_to_db`
зі старого файлу ще один раз завантажуються з попередженням і переносяться до
`[log]`; `rtok config validate` відхиляє ці ключі, бо їх немає в довідковій
схемі.

```toml
# Конфігурація rtok. Кожен прапорець CLI має тут ключ; прапорці та змінні середовища RTOK_* мають пріоритет.
# Пріоритет: типові значення < цей файл < <git root>/.rtok.toml < env < прапорці.
# Документація: docs/config.md. Перевірка: `rtok config validate`. Звідки взялося значення: `rtok config show --sources`.

[core]
enabled     = true                    # false = простий проксі (без бізнес-логіки); HTTP працює до завершення процесу
db_path     = "~/.rtok/rtok.db"       # один файл SQLite, WAL (рішення D8)
archive_dir = "~/.rtok/archive"       # сирі дані для `rtok expand <id>` (рішення D4)
session_env = "CLAUDE_SESSION_ID"     # змінна середовища, з якої береться id сесії, коли його немає в stdin
call_io_inline_bytes = 65536          # тіла MCP/API, більші за це, ідуть в архів (хуки ніколи не архівують)
hook_max_input_bytes = 8388608        # обмеження stdin для rtok hook <event> (8 MiB); понад нього — вихід із кодом 0 без змін, без архівування й гешування (T201)
retain_calls_days    = 30             # 0 = зберігати `calls` назавжди
retain_hook_bodies_days = 3           # тіла stdin хуків очищуються через N днів, рядки лишаються; 0 = доти, доки й `calls` (T352)
retain_archive_days = 90              # тіла архіву неактивних сесій видаляються через N днів; 0 = зберігати (живої сесії — ніколи) (T968)
max_store_mb = 2048                   # db + WAL тримаються під цим значенням скороченням `retain_calls_days` (мінімум 7 днів); 0 = без ліміту (T968)
max_archive_mb = 1024                 # архів тримається під цим значенням видаленням найстаріших тіл неактивних сесій; 0 = без ліміту (T968)

[log]                                 # власний лог rtok (D26); його читає `rtok logs`
path      = "~/.rtok/logs/rtok.log"   # ротовані копії лежать поруч: rtok.log.1 … .5
max_bytes = 1048576                   # ротація понад 1 MiB (validate: ≥ 1024)
files     = 5                         # скільки поколінь зберігати; старіші видаляються, ніколи не архівуються (validate: ≤ 20)
lines     = 200                       # що друкує `rtok logs`, коли --lines не задано
level     = "info"                    # error | warn | info | debug
to_db     = true                      # також записувати рядок `logs` для `rtok otel`
tspin     = "auto"                    # `rtok logs` через tailspin: auto = термінал і tspin у PATH | always | off (T225.1)

[estimator]                           # символів на токен для кожного класу, евристика (точність ще не виміряно); `rtok stats --calibrate` переписує
code  = 3.5
prose = 4.2
json  = 3.0
cjk   = 1.0

# ── інтерфейси ──────────────────────────────────────────────────────────────

[hook]                                # rtok hook <event>
host      = "claude"                  # claude | cursor | copilot | devin | cline — зіставлення полів даних (T10.1, T46.3, T87, T94)
max_ms    = 10                        # м'який бюджет; понад нього подія логується як повільна
slow_warn_s = 300                     # одне попередження про повільну подію на кожні стільки секунд, з кількістю й найгіршим часом; 0 = кожен повільний виклик
fail_open = true                      # будь-яка помилка → `{}` і код виходу 0; false лише для налагодження

[agents]                              # реєстр агентів rtok (T282, D34); див. agents-and-worktrees.md
enabled    = true                     # false = хуки пропускають register/touch/end агента й надсилання повідомлень (облік сесій не змінюється)
idle       = "30m"                    # вікно `live()`: немає `ended_at`, а `last_seen` у межах цього проміжку від поточного моменту
push_bytes = 1024                     # оформлені повідомлення, що надсилаються на кожен UserPromptSubmit/PostToolUse; решта → "and N more" (T288)

[agents.usage]                        # rtok agents usage (T358)
source = "logs"                       # logs = власні файли сесій агентів (Claude Code, Codex, OpenCode, Kilo, Copilot CLI, Gemini CLI); rtok = те, що пройшло через rtok; both
hosts  = []                           # [] = кожен хост; інакше id хостів, напр. ["claude", "codex"]
since  = ""                           # "" = увесь час; дата (2026-09-01, цілі дні в tz) або тривалість (30d)
until  = ""                           # "" = по сьогодні; дата, включно
period = "monthly"                    # monthly | daily: нижня таблиця
by     = "agent"                      # agent | model: за чим групує середня таблиця
tz     = ""                           # зона IANA для меж днів і місяців; "" = системна зона

[agents.usage.dirs]                   # звідки `rtok agents usage` читає власні записи кожного хоста (T358.3); Claude Code і Codex використовують [stats] transcripts_dir / codex_dir
opencode = ["~/.local/share/opencode"] # файли opencode*.db у ньому; незмінений типовий шлях слідує за $XDG_DATA_HOME
kilo     = ["~/.local/share/kilo"]     # файли kilo*.db у ньому; незмінений типовий шлях слідує за $XDG_DATA_HOME
copilot  = ["~/.copilot/session-state"] # */events.jsonl; незмінений типовий шлях слідує за $COPILOT_HOME
gemini   = ["~/.gemini/tmp"]           # */chats/session-*; незмінений типовий шлях слідує за $GEMINI_CLI_HOME
droid    = ["~/.factory/sessions"]     # позначається як unsupported, коли є: Factory не документує поля токенів
pi       = ["~/.pi/agent/sessions"]    # */*.jsonl; незмінений типовий шлях слідує за $PI_CODING_AGENT_SESSION_DIR, інакше $PI_CODING_AGENT_DIR/sessions
kimi     = ["~/.kimi-code/sessions"]   # Kimi Code: */*/agents/*/wire.jsonl; незмінений типовий шлях слідує за $KIMI_CODE_HOME
grok     = ["~/.grok/sessions"]        # позначається як unsupported, коли є: xAI вказує на `grok usage`, який rtok не запускає; слідує за $GROK_HOME
zcode    = ["~/.zcode"]                # позначається як unsupported, коли є: ZCode не документує свої записи сесій
antigravity = ["~/.gemini/antigravity"] # позначається як unsupported, коли є: Google не документує локальні дані Antigravity

[agents.junk]                         # rtok agents junk list|clear: межі віку, захищені шляхи, ваші власні шляхи для сміття
keep_logs_days          = 30          # записи `logs` (задокументовані теки логів агентів), змінені за стільки днів, лишаються; 0-3650
temp_min_age_hours      = 24          # записи `temp`, торкнуті за стільки годин, лишаються; 0-87600
stale_session_days      = 30          # лише з `--kind sessions`: сесії, яких не торкалися довше за стільки днів, є сміттям, єдиний критерій тут час; 0-3650; `--session-days N` на один запуск
stale_worktree_days     = 14          # злитий чистий worktree, який видалив би `rtok worktree gc`, стає сміттям (`stale-worktrees`, review), коли простоює стільки днів; 0-3650
crash_dump_min_age_days = 7           # дамп збою в теці `extra` з kind crash-dumps, старший за стільки днів, безпечно чистити, молодший іде як review; 0-3650
exclude                 = []          # glob-и (~ = домашня тека), яких ніколи не чіпають, як і теки зі збігом, напр. ["~/.claude/debug/keep-*"]; хибний glob зберігає все
extra                   = []          # шляхи, які ви визнаєте сміттям (D36), напр. [{ host = "cursor", kind = "cache", path = "~/Library/Application Support/Cursor/CachedData" }]; kind = cache | temp | logs; host = id хоста або rtok

[mcp]                                 # rtok mcp
tools                   = []          # [] = усі інструменти ввімкнених плагінів; інакше список дозволених; `expand` завжди лишається в переліку (D4)
max_description_tokens  = 60          # перевіряється тестом (T4.1)
max_result_chars        = 20000       # понад це — head/tail + id архіву
http                    = "127.0.0.1:8791"  # `rtok mcp --http` без адреси; тримайте на loopback
# Через HTTP expand доступні лише дані, архівовані цим запущеним сервером; перезапуск скидає доступ.
http_tools              = ["read", "search", "tree"]  # список дозволених для HTTP, замість `tools`; `expand` завжди лишається в переліку; інструменти, що діють від імені агента (`whoami`, `agent_send`, `worktree_add`, …), — ніколи
token                   = ""          # bearer-токен для --http; краще RTOK_MCP_TOKEN; порожній — --http не запускається (T401)
public_url              = ""          # URL тунелю (https://…); лише його хост і origin приймаються ззовні

[proxy]                               # rtok proxy
enabled         = true                # false = простий зворотний проксі (в обхід compress/обліку); НЕ зупиняє HTTP
bind            = "127.0.0.1"
port            = 8790
mode            = "passthrough"       # passthrough | compress
upstream        = "https://api.anthropic.com"      # RTOK_UPSTREAM; ланцюжок за іншим проксі для A/B
openai_upstream = "https://api.openai.com"         # RTOK_OPENAI_UPSTREAM (D11)
gemini_upstream = "https://generativelanguage.googleapis.com"  # RTOK_GEMINI_UPSTREAM (T51.3)
timeout_s       = 600                 # тайм-аут запиту до upstream
max_body_bytes  = 8388608             # ліміт запиту й збережений префікс відповіді (8 МіБ); великі відповіді продовжують передаватися
max_in_flight   = 8                    # загальний ліміт, включно з режимом без lanes; надлишкові запити отримують 429
include_usage   = true                # потокова передача OpenAI: додати stream_options.include_usage, якщо його немає (T11.2)
context_management = false            # лише Anthropic /v1/messages: додати правку clear_tool_uses + бета-заголовок (T51.2, вмикається явно)
dry_run         = false               # --dry-run: вивести фактичні налаштування [proxy] і вийти, не обслуговуючи запити
# TLS: кореневі сертифікати Mozilla webpki (`use_preconfigured_tls`). Корпоративні CA: SSL_CERT_FILE (PEM, curl). Див. «TLS і корпоративні CA».

[proxy.tools_rewrite]                 # T59.5; вимкнено: байти запиту лишаються ідентичними
enabled = false
compact_schema = false                # T686; також перше речення в описах властивостей і зайві default/examples у input_schema; оригінал в архіві (Measurement.ref_id)
max_description_tokens = 60           # 0 = без обрізання; за межею речення; оцінювач Class::Prose
allow = []                            # порожньо = зберігати всі назви, яких немає в deny
deny = []                             # прибрати ці назви з tools[]; подальші виклики все одно пересилаються

[proxy.lanes]                         # T385.1; позначати смугу кожного запиту в журналі обліку (calls.kind); байти лишаються ідентичними
enabled = true                        # false = кожен запит — непозначений api_request; x-rtok-lane і /lane/<name>/ пересилаються як надіслано

[proxy.lanes.bulk]                    # синхронні скрипти; смуга agent слідує глобальним перемикачам, batch і files не переписуються ніколи
compress            = false           # переписування proxy.mode = "compress" (archive, compress, зачистка шуму)
toon                = false           # фільтр toon всередині цього проходу; потрібен compress
tools_rewrite       = false           # proxy.tools_rewrite
context_management  = false           # proxy.context_management
semantic_cache      = false           # plugins.proxy.semantic_cache, читання і запис
flex                = false           # OpenAI service_tier = "flex" on this lane; see [proxy.flex]
timeout_s           = 0               # таймаут читання для цієї смуги; 0 = proxy.timeout_s, але не менше 900 при flex = true (гайд OpenAI щодо Flex бере 15 хв)
upstream            = ""              # базовий URL для кожного запиту цієї смуги, будь-який wire; "" = proxy.upstream / openai_upstream / gemini_upstream
max_in_flight       = 0               # запитів до upstream одночасно; 0 = без ліміту цієї смуги (загальний ліміт діє)
max_queued          = 8               # з max_in_flight: скільки запитів чекають на слот; наступний отримує 429 + Retry-After

[proxy.lanes.embeddings]              # ті самі ключі, що в bulk
compress            = false
toon                = false
tools_rewrite       = false
context_management  = false
semantic_cache      = false
flex                = false
timeout_s           = 0
upstream            = ""
max_in_flight       = 0               # без ліміту цієї смуги; загальний ліміт діє
max_queued          = 8

[proxy.lanes.meta]                    # ті самі ключі, що в bulk (models, підрахунок токенів)
compress            = false
toon                = false
tools_rewrite       = false
context_management  = false
semantic_cache      = false
flex                = false
timeout_s           = 0
upstream            = ""
max_in_flight       = 0               # без ліміту цієї смуги; загальний ліміт діє
max_queued          = 8

[proxy.lanes.internal]                # ті самі ключі, що в bulk (власні виклики моделі rtok)
compress            = false
toon                = false
tools_rewrite       = false
context_management  = false
semantic_cache      = false
flex                = false
timeout_s           = 0
upstream            = ""
max_in_flight       = 0               # без ліміту цієї смуги; загальний ліміт діє
max_queued          = 8

[proxy.batch]                         # файли результатів провайдерського Batch
parse_results       = false           # розібрати отриманий файл результатів у рядок usage на запит; тіло пересилається як є

[proxy.flex]                          # OpenAI Flex tier; which lanes get it is [proxy.lanes.<lane>] flex
force               = false           # overwrite a service_tier the client sent (off: a client value is never changed)
on_429              = "none"          # Flex has no capacity: none = the 429 goes to the client | backoff = retry on Flex | default = retry once on service_tier "auto" (the client's own tier, if force replaced one)
retries             = 3               # backoff only: retries before giving up; at most 5
backoff_ms          = 1000            # backoff only: delay before the first retry, doubled each time, capped at 30 s; a 429's Retry-After (seconds) can lengthen it, up to that cap

[proxy.routing]                       # model routing; only the internal and bulk lanes, never the agent lane
enabled             = false           # off: every call keeps the model the client sent

[proxy.routing.bulk]                  # one rule per lane; it acts only with from, to and gate all set
from                = []              # request models it applies to: an exact id or a prefix ending in * (empty = off)
to                  = ""              # the model that serves a matching call (empty = off)
provider            = ""              # provider slug recorded for a routed call (empty = the wire's own); pair with [proxy.lanes.<lane>] upstream
gate                = ""              # where the quality bench for this swap is kept (a path, URL or run id); empty = off

[proxy.routing.internal]              # same keys as bulk (rtok's own model calls)
from                = []
to                  = ""
provider            = ""
gate                = ""

[web]                                 # rtok web (ті самі дані, що й у rtok tui)
host = "127.0.0.1"                    # --host
port = 3333                           # --port

[tui]                                 # rtok tui (ті самі дані, що й у rtok web)
tab       = ""                        # "" = перша вкладка       (--tab <page>)
tick_secs = 2                         # частота повторного читання моделі, як 2-секундний такт у web (--tick-secs)

[ui]                                  # власні рядки rtok у терміналі; канали й --json лишаються простими
emoji = true                          # емодзі перед рядками статусу/попереджень/помилок (RTOK_UI_EMOJI)
color = true                          # розфарбовувати їх; діють NO_COLOR/CLICOLOR_FORCE (RTOK_UI_COLOR)

[stats]                               # rtok stats
since           = "30d"
format          = "table"             # table | json      (--json)
plugin          = ""                  # "" = усі         (--plugin <id>)
transcripts_dir = "~/.claude/projects"
codex_dir       = "~/.codex/sessions" # логи Codex CLI → ще один рядок `api` (T49.2); OpenCode і Copilot CLI читає `rtok agents usage` ([agents.usage.dirs], T358.3); сховища Cursor не містять кількості токенів (перевірено 2026-09-17), тож їх не читають
calibrate_samples = 30                # на клас          (--calibrate)
baseline        = ""                  # типова назва для --compare; "" = немає
price           = false               # показувати вартість у USD для кожної моделі (--price)
# рядки USD за MTok для --price (T49.1). Джерела, отримано 2026-09-17 (Anthropic claude-fable-5-1,
# claude-opus-5-5 і claude-sonnet-5-5: 2026-10-08):
# Anthropic claude-* рядки: https://platform.claude.com/docs/en/about-claude/pricing
# (вхід / запис у кеш на 5m / читання з кешу / вихід). OpenAI gpt-5 / gpt-5-mini:
# https://platform.openai.com/docs/pricing (вхід із коротким контекстом / кешований вхід /
# вихід; окремої ціни запису немає, тож cache_write = input). Моделі без рядка
# виводять `-`, а не здогадку; додавайте власні датовані рядки так само.
[stats.prices."claude-fable-5-1"]
input = 10.0
cache_write = 12.5
cache_read = 0.25
output = 50.0
[stats.prices."claude-opus-5-5"]
input = 4.0
cache_write = 5.0
cache_read = 0.2
output = 20.0
[stats.prices."claude-sonnet-5-5"]
input = 2.0
cache_write = 2.5
cache_read = 0.1
output = 10.0
[stats.prices."claude-sonnet-5"]
input = 2.0
cache_write = 2.5
cache_read = 0.2
output = 10.0
[stats.prices."claude-haiku-4-5"]
input = 1.0
cache_write = 1.25
cache_read = 0.1
output = 5.0
[stats.prices."gpt-5"]
input = 1.25
cache_write = 1.25
cache_read = 0.125
output = 10.0
[stats.prices."gpt-5-mini"]
input = 0.25
cache_write = 0.25
cache_read = 0.025
output = 2.0
# Рівні Batch і Flex, отримано 2026-10-08: ключ — `<model>@batch` або `<model>@flex`, а `--price`
# рахує usage смуги Batch за рядком `@batch`. Anthropic Batch — знижка 50 % на вхід і вихід,
# множники кешу накладаються зверху (сторінка цін вище; рівня Flex немає). В OpenAI Batch і Flex
# мають однакові ставки (https://developers.openai.com/api/docs/pricing).
[stats.prices."claude-fable-5-1@batch"]
input = 5.0
cache_write = 6.25
cache_read = 0.125
output = 25.0
[stats.prices."claude-opus-5-5@batch"]
input = 2.0
cache_write = 2.5
cache_read = 0.1
output = 10.0
[stats.prices."claude-sonnet-5-5@batch"]
input = 1.0
cache_write = 1.25
cache_read = 0.05
output = 5.0
[stats.prices."claude-sonnet-5@batch"]
input = 1.0
cache_write = 1.25
cache_read = 0.1
output = 5.0
[stats.prices."claude-haiku-4-5@batch"]
input = 0.5
cache_write = 0.625
cache_read = 0.05
output = 2.5
[stats.prices."gpt-5@batch"]
input = 0.625
cache_write = 0.625
cache_read = 0.0625
output = 5.0
[stats.prices."gpt-5@flex"]
input = 0.625
cache_write = 0.625
cache_read = 0.0625
output = 5.0
[stats.prices."gpt-5-mini@batch"]
input = 0.125
cache_write = 0.125
cache_read = 0.0125
output = 1.0
[stats.prices."gpt-5-mini@flex"]
input = 0.125
cache_write = 0.125
cache_read = 0.0125
output = 1.0

[report]                              # rtok report (D24: відображає операторську модель, нічого не обчислює)
format = "md"                         # md; html (T22.2), pdf (T22.3), --ai (T22.4)
out    = ""                           # "" = stdout     (--out <path>)
since  = "30d"                        # наскільки далеко в минуле читає звіт
ai     = false                        # відображення у формі для моделі замість --format (--ai)
budget_tokens = 8000                  # --ai відкидає цілі розділи понад це (T22.4)

[bench]                               # rtok bench
tasks    = "bench/tasks.toml"
runs     = 3
dry_run  = false
timeout_s = 900                       # на один запуск задачі
suite    = ""                         # "" = шість задач T9.1; "graph" = T68.9 з MCP і без; "p28" = T385.9.3 компресор проти lossless
[bench.configs]                       # назва = файл налаштувань, що передається в `claude --settings`
a = "bench/configs/legacy.json"
b = "bench/configs/rtok.json"

[doctor]                              # rtok doctor
settings_path   = "~/.claude/settings.json"
claude_json     = "~/.claude.json"
mcp_json        = ".mcp.json"
probe_timeout_ms = 500                # на кожну перевірку /health вузла проксі
mcp_timeout_ms  = 15000               # на tools/list кожного MCP-сервера (сервери uvx/npx запускаються повільно)
instruction_warn_tokens = 1000        # --instructions: позначати файли, більші за це
instructions    = false               # типово запускати аудит інструкцій (--instructions)

[setup]                               # rtok agents install / remove <host>
dry_run      = false
yes          = false                  # потрібно для --replace
backup       = true                   # <name>.bak-<ts> поруч із кожним файлом, перш ніж setup і remove його змінять
backup_files = 5                      # скільки поколінь .bak-* зберігати на файл; старіші видаляються (0 = зберігати всі)
hook_timeout_s = 5                    # тайм-аут, що записується в кожен запис хука
modes        = []                     # наприклад ["terse", "yagni"]   (--mode)
mcp          = true                   # також зареєструвати MCP-сервер   (--mcp)
proxy        = false                  # також задати base URL           (--proxy)
[setup.paths]                         # хост = "шлях": де застосунок хоста, коли rtok не знаходить його сам (T966; `agents install|update|info` питає і пропонує зберегти)
# напр. claude = "~/.local/bin/claude" # CLI-бінарник або десктопний застосунок (`/Applications/Claude.app`, `claude.exe`)
[setup.claude]
settings_path = "~/.claude/settings.json"
[setup.cursor]
hooks_path    = "~/.cursor/hooks.json"  # сьогодні лише beforeShellExecution (T10.11 підключає PostToolUse)
[setup.codex]
config_path   = "~/.codex/config.toml"
[setup.opencode]
config_path   = "~/.config/opencode/opencode.json"
[setup.kilo]
config_path   = "~/.config/kilo/kilo.json"      # kilo.jsonc об'єднує сам Kilo, rtok його ніколи не переписує
[setup.pi]
extensions_path = "~/.pi/agent/extensions"
tools           = false                     # pi.registerTool для read/search/graph/memory (T70.3)
[setup.omp]
extensions_path = "~/.omp/agent/extensions" # oh my pi: сюди створюється посилання на plugins/pi (T92)
mcp_path        = "~/.omp/agent/mcp.json"
[setup.zcode]
config_path   = "~/.zcode/cli/config.json"
[setup.kimi]
config_path   = "~/.kimi-code/config.toml"  # mcp.json читається поруч
[setup.copilot]
dir           = "~/.copilot"                # mcp-config.json, hooks/rtok.json
[setup.commandcode]
dir           = "~/.commandcode"            # settings.json (ключ hooks), mcp.json
[setup.aider]
config_path   = "~/.aider.conf.yml"         # openai-api-base → rtok proxy (--proxy)
[setup.windsurf]
config_path   = "~/.codeium/windsurf/mcp_config.json"
[setup.zed]
config_path   = "~/.config/zed/settings.json"
[setup.cline]
hooks_path = "~/Documents/Cline/Hooks"
mcp_path = "~/.cline/data/settings/cline_mcp_settings.json" # CLI; розширення використовує файл налаштувань у globalStorage VS Code
[setup.gemini]
dir = "~/.gemini" # settings.json (hooks, mcpServers)
[setup.codewhale]
dir = "~/.codewhale" # config.toml ([[hooks.hooks]]), mcp.json (mcpServers)
[setup.mimo]
config_path   = "~/.config/mimocode/mimocode.json" # mcp (у форматі форку OpenCode)
[setup.antigravity]
plugins_path     = "~/.gemini/config/plugins"          # Antigravity 2.0 / IDE: сюди створюється посилання на plugins/antigravity
cli_plugins_path = "~/.gemini/antigravity-cli/plugins" # сюди готує файли agy plugin install; лише читання
[setup.devin]
config_path   = "~/.config/devin/config.json"  # mcp_config.json читається поруч; Windows: %APPDATA%\devin\
[setup.roo]
mcp_path      = ""                               # порожньо: <Code user dir>/globalStorage/rooveterinaryinc.roo-cline/settings/mcp_settings.json
[setup.junie]
mcp_path      = ""                               # порожньо: ~/.junie/mcp/mcp.json (користувацький файл MCP Junie CLI)
[setup.qwen]
dir           = "~/.qwen"                    # settings.json (хуки, mcpServers); QWEN_HOME переносить каталог
[setup.codebuddy]
dir           = "~/.codebuddy"               # CodeBuddy Code: settings.json (хуки), .mcp.json, plugins/; CODEBUDDY_CONFIG_DIR переносить каталог
workbuddy_dir = "~/.workbuddy"               # WorkBuddy Desktop: ті самі файли (вбудований CodeBuddy Code; неперевірено)
[setup.goose]
config_path   = "~/.config/goose/config.yaml"  # Goose CLI and Desktop: extensions.rtok; Windows: %APPDATA%/Block/goose/config/config.yaml; GOOSE_PATH_ROOT: <root>/config/config.yaml
[setup.droid]
mcp_path      = "~/.factory/mcp.json"        # Factory Droid: користувацький файл MCP-серверів (mcpServers)
[setup.kiro]
mcp_path      = "~/.kiro/settings/mcp.json"      # Kiro IDE and CLI: user-level MCP servers (mcpServers)
[setup.amp]
settings_path = "~/.config/amp/settings.json"    # Amp: user settings; amp.mcpServers holds MCP servers (settings.jsonc if it is the only one)
[setup.warp]
mcp_path      = "~/.warp/.mcp.json"          # Warp: користувацький файл MCP-серверів (mcpServers)
[setup.continue]
config_path   = "~/.continue/config.yaml"        # Continue (VS Code, JetBrains, cn): mcpServers item rtok; Windows: %USERPROFILE%/.continue/; CONTINUE_GLOBAL_DIR moves it
[setup.crush]
config_path   = "~/.config/crush/crush.json" # Crush: global JSON config (mcp.rtok); CRUSH_GLOBAL_CONFIG or XDG_CONFIG_HOME move it
[setup.openhands]
mcp_path      = "~/.openhands/mcp.json"     # OpenHands: користувацький файл MCP-серверів (mcpServers)
[setup.augment]
settings_path = "~/.augment/settings.json"   # Auggie CLI: користувацькі налаштування, mcpServers
[setup.reasonix]
dir           = "~/.reasonix"                # Reasonix: config.toml (MCP-сервери в [[plugins]]); REASONIX_HOME переносить каталог (Windows: %APPDATA%/reasonix)

[expand]                              # rtok expand <id>
max_lines = 0                         # 0 = без обмежень (--lines a-b задається для виклику)
max_rate  = 0.05                      # стеля повторних читань; понад неї звіт позначає стиснення з втратами (T22.5)

[filter]                              # rtok filter --stdin (T10.2)
cmd = ""                              # підказка сімейства команди, коли викликач його знає (--cmd)

[worktree]                            # rtok worktree add | claim | remove | list | gc
enabled = true                        # false: кожна команда `rtok worktree` (і list) каже, що worktree не ввімкнено, MCP не показує жодного інструмента worktree_*, а хуки Claude WorktreeCreate/WorktreeRemove роблять те, що Claude робить без rtok
root = ".rtok/worktrees"              # де `rtok worktree add` створює worktree, як <root>/<repo>-<task>; відносний: у основному checkout; `~` розкривається
seed = true                           # заповнити seed_dirs нового worktree із сусіднього checkout, що збирав їх останнім: copy-on-write клони (APFS, btrfs, XFS, bcachefs, ReFS, Dev Drive), інакше звичайна копія; `rtok worktree add --no-seed` пропускає це один раз
seed_dirs = ["target", "node_modules"] # кеші збірки для засіву, відносно checkout; голе ім'я збігається й на рівень нижче (web/node_modules)
min_free_mib = 2048                    # `rtok worktree add` refuses when the target volume has less free space than this, in MiB (a full disk leaves a half-made worktree); 0 turns the check off

[tasks]                               # адаптери задач (T441); зазвичай задаються для проєкту в .rtok.toml
adapter = "disk"                      # disk | github | gitlab
prefix = ""                           # префікс id задач, 1–8 ASCII-літер (R → R12, R2.1); порожньо: перша літера назви проєкту

[tasks.disk]
dir = "tasks"                         # де лежать Markdown-задачі, відносно кореня проєкту
layout = "auto"                       # files: "R12 - slug.md", виконані в <dir>/done; folders: "<id>/task.md", закрита файлом "<id>/done.md" (plan/); auto: folders, якщо <dir> уже так влаштований

[tasks.github]
repo = ""                             # owner/name; порожньо: remote origin
project = 0                           # номер Projects v2 власника репо: issues потрапляють у нього, поле Status слідує за задачею; 0 = лише issues

[tasks.gitlab]                        # мітки status::in-progress | status::done | status::wont-do; підзадача пов’язана з батьківською (relates_to)
url = "https://gitlab.com"            # https базова URL; задайте для власного інстансу; токен: GITLAB_TOKEN, GITLAB_ACCESS_TOKEN, GL_TOKEN, інакше glab
project = ""                          # group/name або числовий id; порожньо: remote origin

[tasks.export.github]                 # `rtok task export github`: односторонньо віддзеркалити завдання з диска в GitHub Issues
repo = ""                             # owner/name; порожньо: [tasks.github] repo, інакше remote origin

[tasks.export.linear]                 # `rtok task export linear`: односторонньо віддзеркалити завдання з диска в Linear; токен у LINEAR_API_KEY
team = ""                             # короткий ключ команди, наприклад "ENG"; порожньо: експорт не запускається

[otel]                                # експорт OpenTelemetry (D19); вимкнено, доки не визначено endpoint
endpoint      = ""                    # базова URL-адреса OTLP/HTTP, наприклад "http://localhost:4318"; "" = $OTEL_EXPORTER_OTLP_ENDPOINT
headers       = ""                    # "k=v,k2=v2", наприклад "signoz-ingestion-key=…"; "" = $OTEL_EXPORTER_OTLP_HEADERS
service_name  = "rtok"                # service.name ресурсу
content       = false                 # явно увімкніть, щоб надсилати тіла повідомлень, аргументи й результати у спанах
content_bytes = 65536                 # на атрибут; понад це — rtok.archive.id → `rtok expand <id>`
flush_secs    = 5                     # інтервал скидання для proxy / mcp і тайм-аут POST

# ── плагіни ─────────────────────────────────────────────────────────────────

[plugins.measure]
enabled = true

[plugins.cmd]
enabled  = true
rewrite  = true                       # PreToolUse(Bash) → `rtok run -- …`
shell    = ""                         # "" = $SHELL
rules    = "~/.rtok/rules.toml"       # додаткові правила фільтрування; якщо немає → вбудований rules/default.toml
rules_dir = "~/.rtok/rules.d"         # drop-in-файли: кожен *.toml об'єднується після rules у порядку назв (T50.2)
trailer  = true                       # друкувати `[rtok <id8> · N lines]` після скорочення; false = `--no-ref`
trailer_min_lines = 40                # додавати `[rtok <id8> · N lines]` понад це
fail_tail_lines   = 80                # ненульовий код виходу → останні N рядків дослівно
never_wrap = ["rtok", "sudo"]         # список заборонених перших слів; heredoc, `&`, -i завжди пропускаються

[plugins.read]
enabled          = true
default_mode     = "full"             # full | lines | map | signatures
max_chars        = 20000              # понад це — head/tail + id архіву
native_max_bytes = 32768              # поріг відмови для PreToolUse(Read); ніколи не нижче за це
range_max_lines = 300              # T383: нативний Read із limit 1..=N проходить хук; 0 = лише без діапазону
advice           = true               # false = ніколи не відмовляти нативному Read
allow_paths      = []                 # додаткові корені поза cwd
search_max       = 50
search_max_bytes = 1048576             # пошук пропускає файли, більші за це (T55.5)
tree_depth       = 2
delta            = true               # T58.1: змінене повторне читання → unified diff щодо останнього архіву (7.3 % байтів Read, 2026-09-18, `rtok stats --since 90d`)
delta_max_ratio  = 0.6                # повний файл, коли diff не менший за цю частку

[plugins.json_tree]
enabled = false                     # fold repeated nested JSON in old tool results; off until a measurement shows a saving

[plugins.archive]
enabled    = true
keep_turns = 4                        # ніколи не торкатися останніх N ходів
min_tokens = 1500                     # переписувати лише результати інструментів понад це (оцінка)
head_lines = 8
tail_lines = 4
tiers      = false                    # багаторівневе завантаження, вмикається явно (типово вимкнено); специфікація поведінки OpenViking L0/L1/L2 — AGPL-3.0, rtok не вендорить, не лінкує й не запускає її як підпроцес (D6); відкриває нативну реалізацію в T33.2
live_blobs = false                    # зменшувати вкладені дампи JSON + блоки data: у блоках користувача, ніколи results/system/tools/останні 2 ходи (T51.1, вмикається явно)
skills     = false                    # архівувати тіла skills поза keep_turns (T61.2, вмикається явно)

[plugins.proxy]
enabled = true                        # плагін proxy (фіксація використання); сам сервер — це [proxy]

[plugins.inject]
enabled       = true
budget_tokens = 800                   # на хід, усі вставлення разом (рішення D5)
modes_dir     = "~/.rtok/modes"
modes         = []                    # те саме, що й [setup].modes; setup записує сюди

[plugins.guard]
enabled      = true
window_turns = 8
deny_grep_glob = false           # вмикається явно: відмовляти нативним Grep/Glob, спрямовувати до MCP search/tree (T50.4)
grep_symbol = false              # вмикається явно: Grep одного ідентифікатора (`foo`, `fn foo`, `class Foo`, `\bfoo\(`) відхиляється з його 1-5 індексованими визначеннями та кількістю посилань; лише пошук в індексі (T369)
skills = false                   # вмикається явно (Claude Code): відмовляти Skill, чий SKILL.md перевищує skill_max_bytes, із його картою + `expand <id>` (T62.1)
skill_max_bytes = 8192           # тіла такого розміру або менші завантажуються цілком; так само будь-який skill з allowed-tools / model / context / agent у frontmatter

[plugins.memory]
enabled        = true
recall_titles  = 5                    # SessionStart: останні N заголовків + id
recall_tokens  = 200
prompt_recall  = 5                    # UserPromptSubmit: 0 = вимкнено; N = ранжовані заголовки на хід (T69.5)
checkpoint_tokens = 400               # PreCompact → SessionStart(compact)
search_limit   = 5
sync_tokens    = 300                  # rtok memory sync: блок у CLAUDE.md / AGENTS.md (T69.6)
startup_recall = true                 # SessionStart(startup) відновлює найновішу нотатку session:* (T71.2)
handoff        = true                 # MCP-інструмент T59.6 для зведення субагента
spawn_brief        = true             # T130: зведення-вказівник для SubagentStart
spawn_brief_tokens = 300              # T130: бюджет токенів для брифу під час запуску
admit              = true             # T671: mem_save відхиляє надто коротку або дубльовану нотатку
admit_min_chars    = 5                # T671: найкоротше тіло, яке пропускає шлюз
admit_similarity   = 85               # T671: відсоток збігу триграм, що вважається дублем
forget_after_days = 90                # T669: «memory forget» вилучає нотатки, які не читали так довго; 0 — потрібен --days

[plugins.memory.embed]
enabled    = false                    # P29: якщо false — лише FTS5; векторний пошук вмикається явно
provider   = "local"                  # "local" = офлайн-хеш-вектори | "http" = OpenAI-сумісний API ембедингів
model      = "all-MiniLM-L6-v2"       # "http": назва моделі на сервері
dimensions = 384                      # "http": має збігатися з довжиною вектора сервера
hybrid     = true                     # коли ввімкнено: RRF(fts5, knn) для mem_search і prompt_recall; хук читає лише збережені вектори; false = лише knn
url         = ""                      # "http": базовий URL, напр. "http://localhost:11434/v1" (Ollama) або "https://api.openai.com/v1"; текст нотаток надсилається туди
api_key_env = ""                      # "http": назва змінної середовища з ключем; порожньо = без заголовка Authorization
max_chars   = 8000                    # "http": довший текст не надсилається (працює звичайний пошук по нотатках)

[plugins.memory.extract]
enabled         = false                 # rtok memory extract: off by default, a model call can cost money
command         = []                    # program reading the transcript on stdin, one {"title","body"} JSON line out per note
max_notes       = 5                     # notes kept per run
max_input_bytes = 60000                 # transcript bytes sent to the program; the end of the session is kept

[plugins.graph]
enabled    = true
max_tokens = 2000                     # на відповідь; понад це: head + "N more, expand <id>"
impact_tokens = 1500                  # impact: файли згруповано й ранжовано в межах цього бюджету, далі "+K files, M refs not shown"; --all або 0 = усі рядки
map_tokens = 0                        # обмеження карти репозиторію на SessionStart (частка D5 поряд із memory.recall_tokens); 0 = вимкнено, доки не пройде A/B P7
map_rank   = "pagerank"               # порядок карти на SessionStart: refs = посилання на ім'я; pagerank = файли за персоналізованим PageRank, персоналізованим нещодавно редагованими файлами й останнім checkpoint після compact
body_lines = 40                       # symbol(): скільки рядків коду показувати на визначення
auto_index = true                     # true = кожен виклик обходить дерево; false = індексувати один раз, далі `rtok graph index` або спостерігач (файл, позначений хуком як застарілий, до того читається як відсутній)
auto_add_projects = true               # T329.6: реєструвати каталог у реєстрі проєктів, коли там стартує сесія з хуками, створюється або приєднується worktree через `rtok worktree` (під назвою його гілки) або виконується виклик graph MCP; false = реєстр змінюється лише через сторінку й CLI
backend    = "tags"                   # tags | lsp | auto | text: tags = tree-sitter index (default); lsp = language server from PATH, tags when it cannot answer; auto = per project and language, server first, tags second, text search when no grammar parses the project; text = in-process text search only (no call graph, no dead)
lsp_timeout_ms = 40000                # how long one language-server wait (starting, still indexing) may take before the request falls back to tags
backend_by_language = {}              # backend for one language, e.g. { go = "tags", rust = "lsp" }
watch      = "off"                    # off | notify: фонове переіндексування всередині `rtok mcp` (P8d)
auto_link_references = true           # T329.8: йти за посиланнями в маніфестах (Cargo path, npm file:/link:, go replace, Python path, submodules) в інші каталоги, реєструвати й автоматично зв'язувати їх
reference_depth = 3                   # T329.8: скільки рівнів посилань іти від проєкту (A -> B — це 1); досягнення межі показується й логується
max_auto_projects = 20                # T329.8: найбільше проєктів, які посилання можуть додати до реєстру; досягнення межі показується й логується
alerts = true                         # T329.17: сповіщати, коли проєкт області зник, недоступний, має збійний бекенд або зламане посилання маніфесту; показують rtok doctor, graph projects і відповіді graph
health_check_interval_s = 60          # T329.17: секунд між перевірками здоров'я в rtok mcp і rtok web (сповіщення потребує двох поспіль); 0 = вимкнено
live_heat_window_s = 300            # T329.34: секунд, скільки вузол лишається «теплим» на живому полотні графа після виклику (rtok web, rtok tui)
live_max_events_per_s = 50          # T329.34: скільки подій викликів жива панель показує за секунду; лічильники рахують кожен виклик
live_feed_rows = 200                # T329.34: скільки завершених викликів зберігає жива стрічка (1-1000)

[plugins.toon]
enabled  = true
min_rows = 5

[plugins.compress]
enabled = true                        # екстрактивні зведення заархівованого виводу інструментів; працює лише в proxy.mode = "compress"

[plugins.compress.llm]
enabled           = false             # P28: model summaries of archived output via the internal lane; needs proxy.mode = "compress"
url               = ""                # where the compressor posts; empty = this proxy (proxy.bind:proxy.port)
model             = "claude-haiku-4-5"
max_input_bytes   = 65536             # larger archived results keep the extractive view
max_output_tokens = 512
max_spans         = 64                # more must-keep spans than this keep the extractive view
max_in_flight     = 2                 # compressor calls at once
timeout_s         = 60
code              = false             # also summarise diffs and source; off = logs and prose only

[plugins.docs]
enabled        = false                # локальний rustdoc з Cargo.lock + кешований JSON docs.rs (T455)
query_limit    = 5                    # стеля влучань docs_query
snippet_chars  = 400                  # символів на FTS-фрагмент
max_tokens     = 800                  # стеля оцінки всієї відповіді; зайві влучання відкидаються

[plugins.wasm]
enabled = false                      # типово вимкнено; жоден .wasm не завантажується до хоста T32.2 + feature `wasm-host`
dir     = "~/.rtok/plugins"          # шукати *.wasm на один рівень углиб; D6 — цей репозиторій ніколи не вендорить сторонніх плагінів
```

### `[plugins.proxy.semantic_cache]` — кеш відповідей, що вмикається явно (P31)

Типово вимкнено до Gate P31 (нуль хибних влучань на наборі P9). Коли ввімкнено (T31.2), проксі може
віддати попередню відповідь, коли нормалізований промпт достатньо схожий; хибне влучання — це неправильна відповідь, тому
це лишається явно ввімкнюваним. Змінна середовища: `RTOK_PLUGINS_PROXY_SEMANTIC_CACHE_ENABLED=true`.

| Ключ | Типово | Значення |
|-----|---------|---------|
| `enabled` | `false` | Головний перемикач; коли вимкнено, байти проксі лишаються ідентичними |
| `threshold` | `0.99` | Нижня межа косинусної подібності для семантичного рівня |
| `ttl_s` | `300` | TTL запису в секундах |
| `max_messages` | `1` | Пропускати кеш, коли довжина `messages` перевищує це |
| `require_empty_tools` | `true` | Не кешувати ходи з непорожнім `tools[]` |
| `embed_backend` | `"hash"` | `"hash"` = лише прямий рівень до появи embeddings P29 |
| `cache_by_model` | `true` | Розділяти записи кешу за моделлю |
| `cache_by_provider` | `true` | Розділяти записи кешу за провайдером |

```toml
[plugins.proxy.semantic_cache]
enabled = false
threshold = 0.99
ttl_s = 300
max_messages = 1
require_empty_tools = true
embed_backend = "hash"
cache_by_model = true
cache_by_provider = true
```



### `[proxy.lanes]`

Кожен проксійований запит класифікується в смугу, і смуга записується в журнал обліку
(`calls.kind`). Першим вирішує шлях: Batch (`/v1/messages/batches`, `/v1/batches`,
Gemini `:batchGenerateContent`), `files`, `embeddings` і `meta` (`/v1/models`,
`count_tokens`) називають власну смугу. Синхронний чат-виклик — це хід `agent`, якщо
викликач не вказав інакше заголовком `x-rtok-lane: bulk|internal` або префіксом шляху `/lane/<name>/`
(`/lane/bulk/v1/messages`). Обидва вилучаються перед відправленням запиту upstream. Евристик
немає: непозначений запит — це `agent`, смуга, якою був кожен запит до появи смуг.

| Ключ | Тип | Типово | Значення |
|-----|------|---------|---------|
| `enabled` | bool | `true` | `false` записує кожен запит як звичайний `api_request` і пересилає заголовок та префікс без змін |

Смуга agent лишає `calls.kind = api_request`; інші записують `api_request:<lane>`
(`api_request:bulk`, `api_request:batch`, ...). Байти запиту смуга не змінює.

Смуга `agent` таблиці не має: для неї кожен глобальний перемикач вирішує так само, як до
появи смуг (стабільність prompt-кешу). `batch` і `files` її теж не мають: їхні тіла
(Batch JSONL, завантаження) завжди пересилаються без змін, а правка `stream_options` для них
теж пропускається. `bulk`, `embeddings`, `meta` і `internal` читають кожна свою таблицю
`[proxy.lanes.<lane>]`, і всі перемикачі в ній типово вимкнені, тому ці смуги пересилаються
байт у байт, доки ви не ввімкнете одну з них. Перемикач смуги лише звужує глобальний:
переписування виконується на смузі, коли ввімкнені глобальний перемикач *і* перемикач смуги.

| Ключ | Який глобальний перемикач звужує | Типово | Значення |
|-----|--------------------------|---------|---------|
| `compress` | `proxy.mode = "compress"` | `false` | archive, compress і зачистка шуму термінала |
| `toon` | `plugins.toon.enabled` | `false` | фільтр `toon` усередині цього проходу; потрібен `compress` |
| `tools_rewrite` | `proxy.tools_rewrite.enabled` | `false` | переписування описів у `tools[]` |
| `context_management` | `proxy.context_management` | `false` | серверне редагування контексту Anthropic |
| `semantic_cache` | `plugins.proxy.semantic_cache.enabled` | `false` | читання і запис кешу |
| `flex` | немає | `false` | OpenAI `service_tier = "flex"` для викликів chat і responses, див. [`[proxy.flex]`](#proxyflex) |
| `timeout_s` | `proxy.timeout_s` | `0` | таймаут читання цієї смуги в секундах; `0` = `proxy.timeout_s`, але не менше 900 при `flex = true` (настанова OpenAI щодо Flex піднімає таймаут SDK до 15 хвилин: запити Flex частіше впираються в таймаут) |
| `upstream` | `proxy.upstream`, `openai_upstream`, `gemini_upstream` | `""` | базовий URL для кожного запиту цієї смуги, хоч би який wire (шлюз або локальний сервер, що їх розуміє); `""` = власний upstream wire |
| `max_in_flight` | немає | `0` | скільки запитів цієї смуги одночасно в upstream, рахуючи до кінця потоку відповіді; `0` = без ліміту цієї смуги, загальний ліміт усе одно діє |
| `max_queued` | немає | `8` | з `max_in_flight`: скільки запитів чекають на слот; наступний отримує `429` з `Retry-After: 1` і до upstream не доходить |

Кожна смуга має власні слоти й власну чергу; `max_in_flight = 0` вимикає лише ліміт цієї смуги.
Загальний ліміт діє для всіх запитів, зокрема `agent`, plain mode та вимкнених смуг; слот
утримується до завершення потоку відповіді. Запит, якому відмовив будь-який переповнений ліміт,
отримує `rate_limit_error` у форматі Anthropic, на який SDK провайдерів відповідають паузою та
повтором; рядка в `calls` він не пише (до upstream він не дійшов). Переповнена черга смуги також
пише рядок `warn`; загальний ліміт не створює окремий запис для кожної відмови. Коли проксі
вимкнено (`proxy.enabled`, `core.enabled` або `plugins.proxy.enabled` дорівнюють false), ліміти й
черги смуг обходяться, але загальна межа ресурсів залишається. `batch` і `files` не мають
`upstream`: створення, опитування й результати Batch-задачі завжди йдуть до провайдера, якому
вона належить.

Смуга `agent` не має перемикача `flex`: живий хід не змінює рівень, доки клієнт сам
його не попросить.

### `[proxy.batch]`

Спостереження за провайдерським Batch. Самі виклики Batch (створення, опитування, список, скасування, результати)
вже позначені `api_request:batch` через `[proxy.lanes]`, а їхні тіла ніколи не переписуються.

| Ключ | Тип | Типово | Значення |
|-----|------|--------|---------|
| `parse_results` | bool | `false` | Після пересилання файлу результатів записує по одному рядку `usage` на успішний запит: Anthropic `GET /v1/messages/batches/{id}/results` та OpenAI `GET /v1/files/{id}/content`, якщо його рядки — результати Batch (такий виклик отримує мітку `api_request:batch`). Рядки з помилкою, прострочені та пошкоджені пропускаються. Потрібен `[proxy.lanes] enabled`; байти відповіді не змінюються. |

```toml
[proxy.batch]
parse_results = false
```

### `[proxy.flex]`

Обробка Flex в OpenAI тарифікує виклик chat або responses за ставками Batch в обмін на затримку
([OpenAI: Flex processing](https://developers.openai.com/api/docs/guides/flex-processing), перевірено
2026-10-08). За `flex = true` у смузі (`bulk`, `internal`, `embeddings` або `meta`; у `agent`, `batch`
і `files` його немає) rtok задає `service_tier = "flex"` у викликах цієї смуги до `/v1/chat/completions`
і `/v1/responses`. В Anthropic немає рівня Flex, його протокол не зачіпається ніколи.

Заданий клієнтом `service_tier` (`auto`, `default`, `priority`, `flex`) не перезаписується, доки не
задано `force = true`. Якщо поля немає, rtok додає його на початок об'єкта, а всі інші байти
пересилає так, як їх надіслав клієнт.

Коли в Flex немає потужності, OpenAI відповідає `429 Resource Unavailable` і нічого не списує. Код помилки
в настанові не названо, тому rtok вважає так будь-який `429` на запит, якому сам задав Flex. Запит,
у якого Flex задав клієнт, не повторюється: цей `429` обробляє клієнт.

| Ключ | Тип | Типово | Значення |
|-----|------|---------|---------|
| `force` | bool | `false` | Перезаписувати заданий клієнтом `service_tier` значенням `flex` |
| `on_429` | string | `"none"` | `none` віддає `429` клієнту; `backoff` повторює на Flex із подвоюваними паузами; `default` повторює один раз із `service_tier = "auto"` (стандартна обробка, дорожче) |
| `retries` | int | `3` | лише `backoff`: повторів, перш ніж останній `429` піде клієнту; не більше `5` |
| `backoff_ms` | int | `1000` | лише `backoff`: пауза до першого повтору, подвоюється, максимум 30 с |

Поки rtok повторює запит, клієнт чекає й бачить лише підсумкову відповідь. Повтор `default` повертає
`service_tier`, який надіслав клієнт, якщо `force` його замінив, і ставить `auto` лише коли клієнт
його не надсилав. `Retry-After` на `429` (лише секунди; дата й інше ігноруються) подовжує паузу до
більшого з нього та затримки backoff, але не більше 30 с. Якщо він просить більше, чекати не будуть: `backoff`
віддає `429` клієнту, `default` одразу повторює на запасному рівні. `408` не повторюється і доходить до
клієнта без змін. Повтори пишуться в журнал на рівні `warn`.

```toml
[proxy.lanes.bulk]
flex = true

[proxy.flex]
force = false
on_429 = "backoff"
```

### `[proxy.routing]`

Маршрутизація моделей (D9) переводить виклик на дешевшу модель, і лише там, де ризик для якості
мінімальний: власні виклики rtok у смузі `internal` та смуга `bulk`, увімкнена явно
(`docs/research/optimization.md` §4.2). Смуга `agent` не маршрутизується ніколи: живий хід іде
на моделі, яку обрав оператор. `batch` і `files` теж ніколи не переписуються. Типово вимкнено,
правил у поставці немає.

| Ключ | Тип | Типово | Значення |
|-----|------|---------|---------|
| `enabled` | bool | `false` | Головний перемикач; вимкнений — кожен виклик іде на моделі, яку надіслав клієнт |

У `[proxy.routing.bulk]` та `[proxy.routing.internal]` лежить по одному правилу. Правило працює,
лише коли задано всі чотири ключі нижче; недописане правило нічого не робить.

| Ключ | Тип | Типово | Значення |
|-----|------|---------|---------|
| `from` | список рядків | `[]` | Моделі запиту, до яких застосовується правило: точний id або id із `*` наприкінці як префікс (`claude-opus-*`). Модель поза списком іде як є |
| `to` | string | `""` | Модель, яка обслужить відповідний виклик |
| `provider` | string | `""` | Slug провайдера, що записується для маршрутизованого виклику, якщо `to` живе не в провайдера wire; `""` = провайдер wire |
| `gate` | string | `""` | Де зберігається бенч якості для цієї заміни (шлях, URL або id запуску) |

`gate` — це контроль якості I-101. Чи достатньо добре відповідає дешева модель — результат бенчу,
а не те, що проксі може побачити, тому rtok вимагає від оператора спершу прогнати бенч на
реальному навантаженні смуги й назвати його тут; сам він його не читає й не оцінює. Без `gate`
правило вимкнене.

Замінюється лише JSON-поле `model` (тіло серіалізується знову); Gemini вказує модель у шляху,
який маршрутизація не переписує, тож виклик Gemini не маршрутизується ніколи. Заміна відбувається
до запису виклику, тож `calls` і `usage` називають модель, що обслужила виклик (і `provider`,
якщо задано), `rtok stats --price` оцінює смугу за ціною цієї моделі, а рядок журналу виклику
зберігає модель, яку просив клієнт. Кожен маршрутизований виклик отримує також рядок виміру
`model_route`; заощадження токенів він не заявляє, бо заощадження — це долари: порівнюйте
`cost by lane` з `rtok stats --price` на тому самому навантаженні з вимкненою та ввімкненою
маршрутизацією. Правило між провайдерами поєднуйте з `[proxy.lanes.<lane>] upstream` для шлюзу,
що обслуговує `to`; rtok не перекладає між wire, тож `to` має бути моделлю, яку приймає API цього
wire.

```toml
[proxy.routing]
enabled = true

[proxy.routing.bulk]
from = ["claude-sonnet-*"]
to = "claude-haiku-4-5"
gate = "bench/bulk-haiku-2026-10.md"
```

### Ціни для статистики (`[stats.prices]`)

`rtok stats --price` оцінює рядки `usage` з проксі в USD: кожну складову за її
рядком `$` за MTok, `cost` — їхня сума, `saved` — скільки заощадили читання з кешу порівняно
з ціною некешованого входу, — єдина економія, яку можна обчислити лише з рядків `usage`.
Модель без рядка виводить `-` в обох доларових стовпцях (кількість її токенів
однаково виводиться); додайте власний датований рядок, а не вгадуйте. Постачені
рядки взято зі сторінок цін провайдерів 2026-09-17 (джерела в
`crates/rtok-config/default.toml`); перевірте їх знову, якщо ваш рахунок не збігається. `stats.price`
типово вмикає показ `--price` (`RTOK_STATS_PRICE=true` теж працює).

Рядок із ключем `<model>@batch` (або `<model>@flex`) оцінює модель на рівні Batch (або Flex). `--price`
виводить usage викликів смуги Batch під `<model>@batch` і рахує його за цим рядком; без нього друкує `-`,
а не бере стандартну ставку. Виклики, які провайдер повідомив як обслуговані на Flex (`service_tier` у відповіді, записується в
`calls.service_tier`), виводяться під `<model>@flex` і рахуються так само. `rtok stats` і `rtok report` додають таблицю
`lane/tier` з лічильниками кешу, коли трафік ішов поза смугою agent або на рівні, відмінному від `standard` / `default`;
Коли трафік пішов поза смугою agent, `--price` друкує ще й `cost by lane` (у JSON — `cost.lanes`):
долари кожної смуги, де кожен виклик оцінено за моделлю, записаною для нього, тож виклик, переведений
[`[proxy.routing]`](#proxyrouting), оцінюється за моделлю, що його обслужила. `*` позначає смугу,
де була й модель без рядка ціни, тож її сума — нижня оцінка.

### Хост WASM-плагінів (`[plugins.wasm]`)

Плагіни `.wasm` поза деревом (P32, рішення D6). Цей репозиторій пише кожен плагін каталогу з
нуля й **ніколи не вендорить сторонні блоби `.wasm`** — оператори встановлюють їх у
`plugins.wasm.dir` на своїй машині. Типово `enabled = false`: збірки з дерева й типова
конфігурація не завантажують WASM. Хост Wasmi і Cargo feature `wasm-host` з'являться в T32.2; доти
прапорець видно в `rtok config show --sources`, але завантажувача немає.

### Бекенди графа (`[plugins.graph]`)

`backend = "tags"` (типово) відповідає з індексу tree-sitter. `"lsp"` спрямовує `symbol` / `callers` /
`impact` / `outline` / `explore` через мовний сервер із `PATH` і віддає відповідь tags, коли сервер
відповісти не може. `"auto"` обирає режим для кожного проєкту й мови: сервер, якщо для мови проєкту він
встановлений, інакше tags, а якщо жодна граматика не розбирає проєкт — текстовий пошук, і кожна відповідь
повідомляє, який режим відповів. `"text"` (T329.10) закріплює простий текстовий пошук: шаблони з межами
слів для визначень і згадок по файлах, які обходить інструмент `search`, усередині процесу (`rg`, `grep`
і `ssh` не запускаються), без графа викликів, тому `dead` і ланцюжки `to` в `impact` відповідають
«not available in text mode».

`backend_by_language` перевизначає `backend` для однієї мови, яку визначає файл-маркер проєкту: `rust`,
`c`, `typescript`, `dart`, `go`, `python`, `javascript`, `java`, `ruby`, `php`, `elixir`, `swift`
(`go = "tags"` лишає Go на індексі, поки решта працює в `auto`). `lsp_timeout_ms` (типово 40000) — скільки може тривати одне очікування сервера, перш ніж
запит відкотиться на tags. Покрокове налаштування для Rust (rust-analyzer) і Dart (Dart SDK) та формат
відповіді `auto`: `docs/lsp.md`.

### Вивід у терміналі (`[ui]`)

Власні рядки rtok для людини за терміналом — `ok …`, `… started` / `… stopped`, `warning: …`,
`Error: …`, підсумок `graph index`, `--help` — типово мають емодзі й колір:
✅ успіх (зелений), 💡 статус (блакитний), ❗ попередження (жовтий), ❌ помилка (червоний). Рядок з назвою операції отримує її значок (📚 index, 🚀 start, 🛑 stop, 🔗 link, 🧹 remove, …), вирівняний так, що текст після нього починається в одній колонці.

```toml
[ui]
emoji = false   # RTOK_UI_EMOJI=false
color = false   # RTOK_UI_COLOR=false
```

- **Емодзі** потребують `emoji = true` *і* термінала на відповідному потоці.
- **Колір** потребує `color = true` *і* потоку, що приймає колір: термінал, `NO_COLOR`
  не задано, `TERM` не `dumb` — або `CLICOLOR_FORCE` / `FORCE_COLOR`, що примусово вмикають його для каналу.
  `color = false` також вимикає кольори diff, слів стану й рівнів логу.
- Те, що читають агенти, ніколи не змінюється: JSON хуків і MCP, відфільтрований вивід команд, `--json` і
  все, що записується в канал чи файл, лишаються простими, байт у байт.
- Колір `--help` вирішує сам clap (термінал, `NO_COLOR`, `CLICOLOR_FORCE`): довідка виводиться
  ще до читання конфігурації.

## TLS і корпоративні CA

`rtok proxy` і експорт OpenTelemetry використовують одну спільну клієнтську конфігурацію rustls: кореневі сертифікати Mozilla
через `webpki-roots`, передані в reqwest через `use_preconfigured_tls`. Вони
не використовують верифікатор macOS Security.framework. Корпоративні чи приватні CA:
задайте `SSL_CERT_FILE` як шлях до пакета PEM (угода curl). Ці сертифікати
доповнюють набір Mozilla. Якщо змінну задано, відсутній, порожній або
нерозбірний файл зупиняє запуск із шляхом у повідомленні про помилку (як у curl).
Якщо не задано, використовуються лише кореневі сертифікати Mozilla. `rtok hook` ніколи не відкриває TLS.

## OpenTelemetry

`[otel]` вмикає експортер (`docs/otel.md`). Вимкнено, доки `endpoint` або
`OTEL_EXPORTER_OTLP_ENDPOINT` не вкаже колектор; на шляху хука нічого не виконується.

## Таблиця відповідності (прапорці → ключі)

| Підкоманда | Прапорець | Ключ |
|-----------|------|-----|
| глобальний | `--config <path>` | (обирає файл; не ключ) |
| глобальний | `RTOK_HOME` | (обирає каталог; лише змінна середовища, не прапорець clap) |
| читання | `--json` | `stats.format` для `stats`; в інших випадках — дія (сторінка `web::model` як JSON, а не збережений ключ). Для `stats`, `info`, `config show`, `doctor`, `plugins`, `agents list`, `agents sessions`, `agents whoami`, `agents show`, `agents inbox`, `worktree whoami`, `logs`, `demon status`, `otel status` |
| `hook` | `--host` | `hook.host` |
| `proxy` | `--port`, `--upstream`, `--mode`, `--dry-run` | `proxy.port`, `proxy.upstream`, `proxy.mode`, `proxy.dry_run` |
| `web` | `--host`, `--port` | `web.host`, `web.port` (`rtok dashboard` — застаріле написання) |
| `tui` | `--tab`, `--tick-secs` | `tui.tab`, `tui.tick_secs` |
| `stats` | `--since`, `--plugin`, `--compare`, `--calibrate`, `--cache`, `--price` | `stats.since`, `stats.format`, `stats.plugin`, `stats.baseline`, (`--calibrate`, `--cache` — дії; їхній параметр — `stats.calibrate_samples`), `stats.price` (`stats.prices.*` — це дані) |
| `report` | `--format`, `--out`, `--since`, `--ai` | `report.format`, `report.out`, `report.since`, `report.ai` (`report.budget_tokens` обмежує `--ai`) |
| `bench` | `--tasks`, `--runs`, `--dry-run`, `--timeout`, `--suite` | `bench.*` |
| `doctor` | `--instructions` | `doctor.instructions` |
| `agents install` | `--dry-run`, `--yes`, `--mode`, `--mcp`, `--proxy`, `--remove`, `--replace`, `--cli`, `--desktop`, `--all` | `setup.*` (`--remove`, `--replace`, `--cli`, `--desktop`, `--all` — дії) |
| `agents remove` | `--dry-run` | `setup.dry_run` (сама команда — це дія `--remove`) |
| `agents list` | — | читає конфігурації хостів і `<bin> --version` (`--json` — див. рядок «читання») |
| `agents whoami` | — | читає `RTOK_AGENT_ID` і знаходить його через сховище (T283); без ключа, без `setup.*` (`--json` — див. рядок «читання») |
| `worktree whoami` | — | читає `RTOK_AGENT_ID` і `[worktree] root` (T411); власного ключа немає (`--json` — див. рядок «читання») |
| `task init` | `--adapter`, `--prefix` | `tasks.adapter`, `tasks.prefix`: записуються в `.rtok.toml` цієї копії репозиторію (T441.5) |
| `task export` | `--dry-run`, `--json` | `tasks.export.github.repo`, `tasks.export.linear.team` (T441.13.5, T441.13.6); `--dry-run` — на один виклик (без ключа) |
| `task create` / `list` / `status` | `--description`, `--body-file`, `--parent`, `--status`, `--all`, `--force` | для одного виклику (без ключа): яке завдання і які рядки показати; адаптер і префікс вибирає `[tasks]` |
| `agents junk list` / `clear` | `--agent`, `--kind`, `--include review`, `--older-than`, `--session-days`, `--trash`, `--bytes`, `--items`, `--sort`, `--min-size`, `--yes` | на один виклик (без ключа): що один запуск показує або видаляє; `--session-days` це `agents.junk.stale_session_days` на один запуск; `agents.junk.keep_logs_days`, `.temp_min_age_hours`, `.crash_dump_min_age_days`, `.stale_worktree_days`, `.exclude`, `.extra` без прапорця |
| `agents usage` | `--source`, `--host`, `--since`, `--until`, `--daily` / `--monthly`, `--tz` | `agents.usage.source`, `.hosts`, `.since`, `.until`, `.period`, `.tz`, а також `.dirs.<host>` без прапорця (`--unpriced` обирає вигляд одного виклику, `--json` — див. рядок «читання») |
| `agents sessions` | `--all` | (дія: також перелічує завершені сесії; live чи idle визначає `agents.idle`) |
| `agents show` | — | знаходить префікс id через сховище (T284); live чи idle визначає `agents.idle` (`--json` — див. рядок «читання») |
| `agents status` | — | записує текст статусу агента, що викликає (`RTOK_AGENT_ID`), ≤ 120 символів (T284); без ключа |
| `agents send` | `--all-live` | для окремого виклику (без ключа, без `setup.*`): одне повідомлення кожному живому агенту проєкту викликача (T287); що вважати «живим», вирішує `[agents] idle` |
| `agents inbox` | `--unread` | для окремого виклику (без ключа, без `setup.*`): які рядки показує одне читання (T287); `--json` — див. рядок «читання» |
| `expand` | `--lines`, `--grep` (регулярний вираз, із запасним буквальним пошуком; збіги виводяться як `N:line`), `--context N` (рядки навколо кожного збігу grep, вікна об'єднуються через `--`) | для окремого виклику (без ключа); `expand.max_lines` обмежує; `expand.max_rate` — стеля для звіту (T22.5) |
| `filter` | `--cmd` | `filter.cmd` |
| `config init`, `config set`, `memory import`, `graph index` | `--dry-run` | (дія: показує зміну як git diff і нічого не записує) |
| `memory export` | `--project` | для окремого виклику (без ключа): звужує один дамп до нотаток проєкту |

Тест покриття (T12.4) обходить дерево команд clap і падає, якщо непозиційний прапорець
з'являється без ключа в `crates/rtok-config/default.toml`, тож ця таблиця не може непомітно розійтися з кодом.

## Приклади змінних середовища

```bash
RTOK_PROXY_MODE=compress rtok proxy
RTOK_PLUGINS_READ_ALLOW_PATHS=/opt/src,/srv/lib rtok mcp
RTOK_PLUGINS_WASM_ENABLED=true rtok config show --sources
RTOK_STATS_SINCE=7d rtok stats
RTOK_CONFIG=./ci-config.toml rtok bench --dry-run
```

## Налагоджувальний лог (`RUST_LOG`)

`[log]` — це лог оператора: файл із налаштованим рівнем плюс рядки `logs`. Для налагодження
є другий потік, лише в stderr, за фасадом `log` і `env_logger` (T225). Він вимкнений,
доки не задано `RUST_LOG`, тож типово жоден запуск хука, `mcp` чи `proxy` не виводить нічого нового:

```bash
RUST_LOG=rtok=debug rtok stats                       # argv, потім кожен рядок [log] у момент запису
RUST_LOG=rtok::log=info rtok proxy                   # лише віддзеркалений потік [log]
RUST_LOG=rtok=debug RUST_LOG_STYLE=never rtok mcp    # без кольору; stdout лишається каналом MCP
```

Віддзеркалення ігнорує `[log] level`: файл зберігає `info`, а stderr показує те, що запитує `RUST_LOG`.
Змінна називається `RUST_LOG`, а не `RTOK_LOG`: назви `RTOK_<SECTION>_<KEY>` належать
рівню змінних середовища вище, і `RTOK_LOG` конфліктувала б із таблицею `[log]`.

`just logs [flags]` відкриває файл у [tailspin](https://github.com/bensadeh/tailspin) (`tspin`,
закріплений у `mise.toml`), який підсвічує рівні, дати, числа й шляхи; `just logs -f` стежить
за ним. Налагоджувальний потік передається каналом так само: `RUST_LOG=rtok=debug rtok stats 2>&1 >/dev/null | tspin`.

`rtok logs` і `rtok logs watch` самі використовують tailspin (T225.1). З типовим `[log] tspin = "auto"`
пронумеровані рядки проходять через `tspin --print`, коли stdout — термінал, а `tspin`
є в `PATH`; `"always"` робить так і для каналу, `"off"` зберігає власні кольори rtok. Без `tspin`
вивід лишається таким, як був. `rtok logs export` і `--json` ніколи через нього не проходять.

## Чому один файл, а не лише прапорці

Хуки запускає хост із фіксованим командним рядком; єдиний спосіб їх налаштувати — це
файл. Проксі й MCP-сервер працюють годинами; перезапускати їх заради зміни прапорця —
це регресія. А бенчмарку потрібні дві повні відтворювані конфігурації — тобто по файлу
на конфігурацію, а не історія оболонки.

## Застарілі ключі

`[dashboard]` переноситься до `[web]` (T21.3). `core.inject_budget_tokens` переноситься до
`plugins.inject.budget_tokens` (T12.1). `core.log_file` / `log_level` / `log_to_db` переносяться до
`[log].path` / `level` / `to_db` (T24.5, D26). Кожен виводить одне попередження під час завантаження, а потім
відкидається; `rtok config validate` відхиляє їх, бо їх немає в довідковій схемі.

`setup.restart_prompt_timeout_seconds` вилучено (T138): запитання про перезапуск після встановлення — це
підтвердження `inquire`, яке чекає на відповідь (Enter, Esc або Ctrl-C означають «ні»). Файл, який досі задає
цей ключ, отримує попередження про невідомий ключ під час завантаження й помилку від `rtok config validate`.
