---
lang: ru
---

# Плагин read

![Агент либо вызывает инструменты MCP read, search и tree, которые разрешают путь через защиту корня и отвечают контуром, указателем дедупликации, diff или ограниченным телом, либо срабатывает хук PreToolUse для Read, который отказывает в большом нативном Read в пользу rtok read](../../../architecture/plugins/read.svg)

Источник: [`read.d2`](../../../architecture/plugins/read.d2). Документация плагина: [README](../../../../src/plugins/read/README.md). Оглавление: [все диаграммы](../README.md).

`read` заменяет файловые инструменты хоста тремя инструментами MCP (`read`, `search`, `tree`), вывод которых приспособлен под токены: контуры (outline) вместо целых файлов, однострочный указатель для файла, который у модели уже есть, diff для изменившегося файла и обрезка с id архива для всего крупного. Хук PreToolUse направляет хост с нативного `Read` большого файла к этим инструментам. Каждое сокращение обратимо без потерь через `expand <id>`.

## Пошагово

1. **Приходит вызов инструмента.** `rtok mcp` направляет `read`, `search` и `tree` в `plugins::read::read`, `search::search` и `search::tree` (`src/mcp.rs`).
2. **Сначала работает защита корня.** `resolve_with` нормализует путь (следуя симлинкам), отказывает в файлах с учётными данными, когда сервер доступен по HTTP, и принимает только пути под cwd, `[plugins.read] allow_paths` и (в `rtok mcp`) соседними рабочими деревьями и корнями клиента `roots/list` (`roots::dynamic`). Всё прочее — ошибка, которая называет путь.
3. **Режим выбирает вид.** `read_with` читает файл и выбирает по `mode` (по умолчанию `[plugins.read] default_mode`): `full` или `lines` с `range` нумерует строки; `map` и `signatures` вызывают `outline::render` (теги tree-sitter, заголовки markdown или первые строки плюс примечание для неизвестного языка); `stripped` убирает узлы комментариев (`outline::stripped`); `density:X` вызывает `density::render`, который оставляет строки с самыми редкими словами; `diff` — это `full` с принудительным путём дельты.
4. **Проверка повторного чтения.** Ключ — `cache::key(path, mode, range)`. `cache::hit` сравнивает новые байты с архивной копией, названной в `read_cache`. Равные байты возвращают `unchanged id <8-char id>` (строка `dedup`). Изменённые байты возвращают унифицированный diff плюс `previous <id>` и `expand <id>`, когда diff меньше `delta_max_ratio` от файла (строка `delta`).
5. **Дедупликация в пределах сессии.** Иначе `plugin::identical_result` ищет тот же sha256 во всей сессии и возвращает указатель вместо тела (строка `dedup`). `cache::remember` затем архивирует байты и записывает строку `read_cache`.
6. **Ограничение.** `cap` или `cap_recording` обрезает тело длиннее `max_chars`, архивирует полный текст и оставляет `archived <id>` в месте обрезки. `Measurement` записывается для `map`, `signatures`, `density` и `stripped` (сырой файл против возвращённого текста), а также для `read_cap`, `search_cap` и `tree_cap`, когда произошла обрезка; не более одной строки на вызов.
7. **Search и tree.** `search` и `tree` обходят дерево через `ignore::WalkBuilder` (учитывается `.gitignore`, пропускаются `.git` и связанные рабочие деревья) и возвращают строки `path:line: snippet` или `path size`, ограниченные `search_max` / `tree_depth`, затем через `cap_recording`.
8. **Хук даёт советы.** На PreToolUse для `Read` `hook::pre_tool` ничего не возвращает, если `advice` выключен или файл больше `native_max_bytes`. Нативный `Read` с `limit` в пределах `range_max_lines` проходит, потому что `Edit` хоста нуждается в нативном `Read`. Если в последних пяти строках PostToolUse есть `Edit` или `Write` этого пути, вызов проходит либо, при включённом `delta` и закэшированном чтении, отклоняется в сторону `read(mode=diff)`. Иначе он отклоняется с точным именем инструмента хоста для инструмента `read` rtok. Каждый отказ записывает строку стоимости `read`/`deny`.
9. **Инвалидация кэша.** `Plugin::post_tool` вызывает `cache::invalidate`, который очищает кэшированный ключ для изменённого пути только когда `delta` выключен; при включённом `delta` устаревшая запись — это то, относительно чего вычисляется diff.

## Где это находится

| Блок | Код |
| --- | --- |
| rtok hook, rtok mcp | `src/hooks/`, `src/mcp.rs` (`read_file`, `search_files`, `tree_files`) |
| Совет по Read | `src/plugins/read/hook.rs` (`pre_tool`, `recently_edited`, `deny`, `WINDOW_TOOL_CALLS`) |
| Защита корня | `src/plugins/read/mod.rs` (`resolve_with`), `src/plugins/read/roots.rs` (`dynamic`, `client_roots`) |
| Переключение режима | `src/plugins/read/mod.rs` (`read_with`) |
| Контур (outline) | `src/plugins/read/outline.rs` (`render`, `stripped`, `tags`) |
| Плотность (density) | `src/plugins/read/density.rs` (`parse`, `render`) |
| cache::hit, delta | `src/plugins/read/cache.rs` (`hit`, `remember`, `invalidate`, `DEDUP_PREFIX`) |
| identical_result | `src/plugin.rs` (`identical_result`) |
| Ограничение | `src/plugins/read/mod.rs` (`cap`, `cap_recording`, `cap_impl`) |
| search / tree | `src/plugins/read/search.rs` (`search`, `tree`, `text_files`) |
| Манифест | `src/plugins/read/mod.rs` (`ReadPlugin`: `Surface::Mcp`, `Surface::Hook`) |
| Конфигурация | `crates/rtok-config/` (`[plugins.read]`) |
| Таблицы | `crates/rtok-store/` (`read_cache`, `archive`, `measurements`) |

## Инварианты

- Сначала защита корня: никакого доступа к файловой системе за пределами разрешённых корней; `allow_paths = [""]` ничего не даёт.
- Без потерь: ограниченный или дедуплицированный результат всегда несёт id, который разрешает `expand`.
- Ответ дедупликации короче тела, которое он заменяет; `identical_result` ничего не возвращает, когда указатель не был бы короче.
- Хук отказоустойчив: ошибка хранилища, неизвестный хост или нечитаемый файл пропускают `Read`, а нативный `Read` с диапазоном никогда не отклоняется.
- Каждый отказ записывается как затрата, а не экономия, поэтому `rtok stats --plugin read` вычитает его.
- Описания инструментов остаются в пределах своего оценочного бюджета токенов (тест в наборе T4.1).

## Связанное

[Обзор](../overview.md) · [Контракт плагина](../plugin-contract.md) · [Ход хуков](../hook-flow.md) · [Ход MCP](../mcp-flow.md) · [archive](archive.md) · [graph](graph.md)
