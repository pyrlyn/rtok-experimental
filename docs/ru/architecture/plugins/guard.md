---
lang: ru
---

# Плагин guard

![guard: PostToolUse архивирует результаты только для чтения; PreToolUse отказывает в повторном вызове, в слишком большом теле навыка или в Grep, похожем на символ](../../../architecture/plugins/guard.svg)

Источник: [`guard.d2`](../../../architecture/plugins/guard.d2). Документация плагина: [README](../../../../src/plugins/guard/README.md). Оглавление: [все диаграммы](../README.md).

`guard` не даёт модели платить дважды за один и тот же результат. После вызова инструмента только для чтения он архивирует результат и запоминает ключ; если идентичный вызов приходит снова в пределах `window_turns`, он отклоняет вызов с указателем на архивную копию. Два добровольных пути используют тот же шов `PreToolUse`: сжатие слишком большого `SKILL.md` и ответ на `Grep` с одним идентификатором из индекса символов.

## Пошагово

1. **Вызов инструмента приходит.** Claude Code запускает `PreToolUse`; pi и OpenCode запрашивают `rtok guard check` (`guard::check`), который строит то же событие `PreToolUse` и возвращает JSON `{"allow": ...}`. `GuardPlugin::pre_tool` диспетчеризует по имени инструмента.
2. **Навык.** `skill::digest` возвращает `None`, пока `[plugins.guard] skills` не равно true. Он разрешает `SKILL.md` только по имени навыка (`resolve` отвергает `/`, `\` и `..`), а `decide` позволяет навыку загрузиться целиком, если тело укладывается в `skill_max_bytes` или его frontmatter называет `allowed-tools`, `model`, `context` или `agent`. Иначе он архивирует тело и отклоняет вызов с картой заголовков и `[rtok <id> · N lines · expand: rtok expand <id>]`, записывая `Measurement { kind: "skill" }`.
3. **Grep.** При включённой функции `graph` и `grep_symbol = true` `grep_symbol::answer` классифицирует шаблон через `symbol_name`, ищет имя в индексе через `symbol_defs` (никогда не индексируя на пути хука) и отклоняет вызов с определениями, когда их от одного до пяти и каждое всё ещё совпадает со строкой файла. Всё остальное передаётся в `native_redirect`, который при `deny_grep_glob = true` и включённом плагине `read` отклоняет `Grep`/`Glob` и указывает на MCP `search` / `tree` (`kind: "native_deny"`).
4. **Повторный Read или Bash.** `cache_key` строит ключ: `read\t<path>` (плюс суффикс диапазона для `offset`/`limit`/`pages`) или `bash\t<cwd>\t<normalised command>` только для команды, которая лишь читает, с суффиксом id субагента, если он есть. `get_read_cache` находит прежнее попадание, `calls_since` измеряет его возраст относительно `window_turns`, а `archive_size` подтверждает, что оригинал ещё существует. Отказ: `duplicate; rtok expand <id>`, с `Measurement { kind: "guard" }`, в котором указано число сэкономленных байт.
5. **PostToolUse сохраняет.** Для ключевого вызова `post_tool` архивирует ответ через `put_archive` и записывает ключ через `put_read_cache`. Тело больше `ARCHIVE_CAP_BYTES` не архивируется, и его ключ очищается, чтобы никакой поздний отказ не указывал на пустоту.
6. **PostToolUse инвалидирует.** Изменяющий Bash, Edit или Write не получает ключа: он очищает ключи `bash`, а также либо все ключи `read` (для Bash или для Edit/Write пути без стабильной идентичности), либо один ключ `read\t<path>`. Оставленный переход `cd` также очищает ключи `bash`.
7. **Загрузки навыков.** `skill::note_load` записывает каждую загрузку в кэш чтения и, когда тот же навык загружается снова до сжатия, возвращает одну строку добавленного контекста. `PreCompact` и `SessionStart` с `source == "compact"` вызывают `skill::forget_loads`.

## Где это находится

| Блок | Код |
| --- | --- |
| `GuardPlugin::pre_tool` / `post_tool` / `pre_compact` / `session_start` | `src/plugins/guard/mod.rs` (`impl Plugin for GuardPlugin`) |
| `rtok guard check` | `src/cli/guard.rs`, `src/plugins/guard/mod.rs` (`check`) |
| `cache_key`, `read_only`, `norm_cmd` | `src/plugins/guard/mod.rs`, `src/plugins/guard/sed.rs` (`script_read_only`) |
| `native_redirect` | `src/plugins/guard/mod.rs` |
| `skill::digest`, `note_load`, `forget_loads` | `src/plugins/guard/skill.rs` |
| `grep_symbol::answer` | `src/plugins/guard/grep_symbol.rs` (`answer`, `symbol_name`) |
| Кэш чтения, архив, измерения | `crates/rtok-store/` (таблицы `read_cache`, `archive`, `measurements`) через `Ctx` |

## Инварианты

- Отказ только тогда, когда архивный оригинал существует (`archive_size` ненулевой); иначе вызов выполняется.
- Ключ получает только Bash, который лишь читает; cwd и каждый переход `cd` входят в ключ, а Bash после относительного перехода `cd` не имеет ключа.
- Путь хука выполняет один индексированный поиск и проверку существования файла и никогда не читает тело архива, укладываясь в бюджет хука 10 мс.
- Отказ `guard` или `skill` записывает `Measurement`; строки `grep_symbol` и `native_deny` учитывают событие, но не заявляют экономии.
- Навык, у которого во frontmatter есть ключ хоста, или любая ошибка разбора или чтения, загружается целиком.
- Ключи конфигурации: `[plugins.guard] enabled`, `window_turns`, `grep_symbol`, `skills`, `skill_max_bytes`, `deny_grep_glob`.

## Связанное

[Обзор](../overview.md) · [Контракт плагина](../plugin-contract.md) · [плагин read](read.md) · [плагин archive](archive.md) · [плагин graph](graph.md)
