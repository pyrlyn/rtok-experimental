---
lang: uk
---

# Плагін guard

![guard: PostToolUse архівує результати лише для читання; PreToolUse відмовляє в повторному виклику, завеликому тілі скілу або Grep, схожому на символ](../../../architecture/plugins/guard.svg)

Джерело: [`guard.d2`](../../../architecture/plugins/guard.d2). Документація плагіна: [README](../../../../src/plugins/guard/README.md). Індекс: [усі діаграми](../README.md).

`guard` не дає моделі двічі платити за той самий результат. Після виклику інструмента лише для
читання він архівує результат і запам'ятовує для нього ключ; якщо такий самий виклик повертається
протягом `window_turns`, він відхиляє виклик із вказівником на архівну копію. Два опційні шляхи
використовують той самий шов `PreToolUse`: згортання завеликого `SKILL.md` і відповідь на
`Grep` з одним ідентифікатором з індексу символів.

## Покроково

1. **Надходить виклик інструмента.** Claude Code запускає `PreToolUse`; pi і OpenCode
   викликають `rtok guard check` (`guard::check`), який будує ту саму подію `PreToolUse` і
   повертає JSON `{"allow": ...}`. `GuardPlugin::pre_tool` розподіляє за іменем інструмента.
2. **Скіл.** `skill::digest` повертає `None`, якщо не ввімкнено `[plugins.guard] skills = true`.
   Він визначає `SKILL.md` лише за іменем скілу (`resolve` відхиляє `/`, `\` і `..`), а `decide`
   дозволяє скілу завантажитися повністю, якщо тіло вміщується в `skill_max_bytes` або його
   frontmatter називає `allowed-tools`, `model`, `context` чи `agent`. Інакше він архівує тіло і
   відмовляє з картою заголовків та `[rtok <id> · N lines · expand: rtok expand <id>]`, записуючи
   `Measurement { kind: "skill" }`.
3. **Grep.** За наявності фічі `graph` і `grep_symbol = true` `grep_symbol::answer` класифікує
   шаблон через `symbol_name`, шукає ім'я в індексі через `symbol_defs` (ніколи не індексуючи на
   шляху хука) і відмовляє з визначеннями, коли їх від одного до п'яти і кожне досі збігається з
   рядком свого файлу. Усе інше передається до `native_redirect`, який за `deny_grep_glob = true`
   і ввімкненого плагіна `read` відхиляє `Grep`/`Glob` і вказує на інструменти MCP `search` /
   `tree` (`kind: "native_deny"`).
4. **Повторний Read або Bash.** `cache_key` будує ключ: `read\t<path>` (плюс суфікс діапазону для
   `offset`/`limit`/`pages`) або `bash\t<cwd>\t<normalised command>` лише для команди, яка тільки
   читає, з суфіксом id субагента, якщо він є. `get_read_cache` знаходить попереднє влучання,
   `calls_since` вимірює його вік відносно `window_turns`, а `archive_size` підтверджує, що
   оригінал досі існує. Відмова має вигляд `duplicate; rtok expand <id>` з
   `Measurement { kind: "guard" }`, що несе зекономлені байти.
5. **PostToolUse зберігає.** Для виклику з ключем `post_tool` архівує відповідь через
   `put_archive` і записує ключ через `put_read_cache`. Тіло понад `ARCHIVE_CAP_BYTES` не
   архівується, а його ключ очищається, щоб жодна пізніша відмова не вказувала на порожнечу.
6. **PostToolUse інвалідує.** Змінювальний Bash, Edit або Write не має ключа: він очищає ключі
   `bash`, а також або кожен ключ `read` (для Bash чи для Edit/Write шляху без стабільної
   ідентичності), або один ключ `read\t<path>`. Залишений перехід `cd` теж очищає ключі `bash`.
7. **Завантаження скілу.** `skill::note_load` записує кожне завантаження в кеш читання і, коли той
   самий скіл завантажується знову до стиснення, повертає один рядок доданого контексту.
   `PreCompact` і `SessionStart` з `source == "compact"` викликають `skill::forget_loads`.

## Де це живе

| Блок | Код |
| --- | --- |
| `GuardPlugin::pre_tool` / `post_tool` / `pre_compact` / `session_start` | `src/plugins/guard/mod.rs` (`impl Plugin for GuardPlugin`) |
| `rtok guard check` | `src/cli/guard.rs`, `src/plugins/guard/mod.rs` (`check`) |
| `cache_key`, `read_only`, `norm_cmd` | `src/plugins/guard/mod.rs`, `src/plugins/guard/sed.rs` (`script_read_only`) |
| `native_redirect` | `src/plugins/guard/mod.rs` |
| `skill::digest`, `note_load`, `forget_loads` | `src/plugins/guard/skill.rs` |
| `grep_symbol::answer` | `src/plugins/guard/grep_symbol.rs` (`answer`, `symbol_name`) |
| Кеш читання, архів, вимірювання | `crates/rtok-store/` (таблиці `read_cache`, `archive`, `measurements`) через `Ctx` |

## Інваріанти

- Відмова лише тоді, коли архівний оригінал існує (`archive_size` ненульовий); інакше виклик
  виконується.
- Ключ мають лише Bash, який тільки читає; до ключа входять cwd і кожен перехід `cd`, а Bash за
  відносним переходом `cd` ключа не має.
- Шлях хука робить один індексований пошук плюс перевірку існування файлу і ніколи не читає тіло
  архіву, лишаючись у бюджеті хука 10 мс.
- Відмова `guard` або `skill` записує `Measurement`; рядки `grep_symbol` і `native_deny`
  рахують подію, але не заявляють про економію.
- Скіл, чий frontmatter містить ключ хоста, або будь-яка помилка розбору чи читання, завантажується
  цілком.
- Ключі конфігурації: `[plugins.guard] enabled`, `window_turns`, `grep_symbol`, `skills`,
  `skill_max_bytes`, `deny_grep_glob`.

## Пов'язане

[Огляд](../overview.md) · [Контракт плагіна](../plugin-contract.md) · [плагін read](read.md) · [плагін archive](archive.md) · [плагін graph](graph.md)
