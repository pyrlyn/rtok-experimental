---
lang: ru
---

# Плагин memory

![Заметки поступают через mem_save, префикс remember, экстрактор, который включают явно, или контрольную точку checkpoint, проходят шлюз редактирования и допуска в SQLite notes с FTS5 и ссылками на файлы, а возвращаются как ограниченный индекс заголовков из хуков SessionStart, UserPromptSubmit и PreCompact либо как результаты инструментов mem_search, mem_pack и mem_get](../../../architecture/plugins/memory.svg)

Источник: [`memory.d2`](../../../architecture/plugins/memory.d2). Документация плагина: [README](../../../../src/plugins/memory/README.md). Оглавление: [все диаграммы](../README.md).

`memory` — это локальное хранилище заметок без вызовов модели. Заметки поступают от агента (`mem_save`), от человека (промпты `remember:`), от механического наблюдателя за вызовами инструментов или от экстрактора, который включают явно и который запускает программу, названную пользователем. Recall подставляет только индекс заголовков, с ценой в токенах для каждого тела, поэтому модель платит за тело, только когда просит его (`mem_get`, `mem_pack`). Ничего никогда не удаляется: заметки выводятся из обращения, заменяются или закрепляются.

## Пошагово

1. **Сохранение.** MCP `mem_save` вызывает `mem_save_gated`. `strip_private` заменяет каждый фрагмент `<private>`...`</private>` на `[REDACTED]`; пустой заголовок или тело отвергаются.
2. **Шлюз допуска.** С включённым `[plugins.memory] admit` `admit::judge` отказывает телу короче `admit_min_chars` и телу, которое похоже на сохранённую заметку проекта по триграммам не менее чем на `admit_similarity` процентов. Он возвращает `{"refused": "too_short"|"duplicate", "id": <stored>}`. Сохранение с существующим `(project, kind, title)` — это ревизия и всегда проходит.
3. **Upsert и ссылки.** `save_scoped` вызывает `upsert_note`: заголовок — ключ темы, поэтому повтор обновляет строку (старый заголовок и тело уходят в `note_versions`) и снимает tombstone. Индекс `notes_fts` поддерживается триггерами. `files::link_note` записывает существующие пути файлов относительно корня, которые называет тело, в `note_files`, а `upsert_note_embedding` обновляет строку вектора для добровольных настроек `embed`.
4. **Другие писатели.** Первая строка промпта, начинающаяся с `remember:`, сохраняется как вид `user` через `remember_save` (с редактированием, но без шлюза). `rtok memory extract` запускает `extract::run`, который передаёт отредактированный хвост транскрипта в `[plugins.memory.extract] command` и сохраняет каждого кандидата как обычную заметку через тот же шлюз сохранения; нужны `extract.enabled`, и ни один хук его не вызывает. `checkpoint.rs` плагина inject обновляет заметку контрольной точки PreCompact и SessionEnd.
5. **Наблюдение.** На PostToolUse `observe::capture` пропускает инструменты `mem_*`, строит типизированный заголовок, очищенное повествование не длиннее 400 символов и файлы из входных данных инструмента, и вставляет одну строку в `observations` (и `observation_files`), дедуплицируя по хешу сессии, инструмента и повествования. Сырой вывод инструмента не копируется.
6. **Recall через хук.** SessionStart запускает `recall`: `list_note_titles` для проекта (закреплённые сначала), затем `render_title_index`, который держит индекс в пределах `recall_tokens`, отбрасывая самые новые записи. UserPromptSubmit запускает `prompt_recall` по первым 24 словам: `search_notes` (FTS или сохранённая гибридная ветвь, когда включены `embed.enabled` и `embed.hybrid`), `files::recall_hits` объединяет заметки, связанные с файлами, которые читала сессия (`read_cache`), или которые называет промпт, и за ними следуют не более двух строк `obs`. PreCompact добавляет `compact_context` (заголовки плюс наблюдения этой сессии) к контексту хоста, а SubagentStart добавляет `handoff::build_brief`, когда включён `spawn_brief`.
7. **Recall через инструмент.** `mem_search` — это FTS5 bm25, ранжированный повторно по использованию, возрасту, закреплению и проекту. `mem_pack` упаковывает то же ранжирование в бюджет токенов, уровень за уровнем (абстракт, первый абзац, тело), пропуская уровень, который не помещается. `mem_get` возвращает тело (выведенная из обращения заметка несёт строку `retired <ts>`) и учитывает использование. `mem_update` выводит из обращения, заменяет или закрепляет.
8. **Каждый путь измеряется.** `recall`, `prompt_recall`, `pre_compact`, `mem_pack`, `mem_get`, `observe` и `brief` записывают строки `Measurement`, у которых «до» — это тела, которые заменил индекс.

## Где это находится

| Блок | Код |
| --- | --- |
| Поверхности | `src/mcp.rs` (`mem_save`, `mem_pack`, `mem_get`, `mem_update`), `src/hooks/mod.rs` (PreCompact), `src/cli/memory.rs` |
| strip_private | `src/plugins/memory/mod.rs` (`strip_private`, `redact_private`) |
| admit::judge | `src/plugins/memory/admit.rs` (`judge`, `Verdict`) |
| upsert_note, путь сохранения | `src/plugins/memory/mod.rs` (`mem_save_gated`, `save_scoped`) |
| files::link_note | `src/plugins/memory/files.rs` (`link_note`, `recall_hits`, `existing_rels`) |
| Префикс remember: | `src/plugins/memory/mod.rs` (`remember_save`) |
| extract::run | `src/plugins/memory/extract.rs` (`run`, `run_configured`, `CommandExtractor`) |
| observe::capture | `src/plugins/memory/observe.rs` (`capture`, `synthesize`), `scrub.rs` |
| Контрольная точка (checkpoint) | `src/plugins/checkpoint.rs` (`save`, `write`), используется в `src/plugins/inject/` |
| recall, prompt_recall | `src/plugins/memory/mod.rs` (`recall`, `prompt_recall`, `render_title_index`, `compact_context`) |
| build_brief | `src/plugins/memory/handoff.rs` (`build_brief`, `handoff`) |
| mem_search, mem_pack, mem_get, mem_update | `src/plugins/memory/mod.rs`, `src/plugins/memory/pack.rs` (`pack_notes`) |
| Импорт, экспорт, синхронизация | `src/plugins/memory/import.rs`, `export.rs`, `sync.rs` (только CLI, на диаграмме не показано) |
| Таблицы | `crates/rtok-store/` (`notes`, `notes_fts`, `note_versions`, `note_embeddings`, `note_files`, `observations`, `observations_fts`, `observation_files`, `read_cache`, `measurements`) |
| Конфигурация | `crates/rtok-config/` (`[plugins.memory]`, `[plugins.memory.embed]`, `[plugins.memory.extract]`) |

## Инварианты

- Никаких вызовов модели: заметки приходят от агента, от человека, от механического наблюдателя или от команды, названной пользователем.
- Жизненный цикл без потерь: вывод из обращения, замена и закрепление, но никогда не удаление; тело выведенной из обращения заметки остаётся читаемым через `mem_get`.
- Фрагменты `<private>` редактируются до того, как что-либо сохраняется, а повествования наблюдений очищаются до обрезки.
- Одна строка на `(project, kind, title)`; upsert с тем же телом не записывает версию.
- Recall вставляет только заголовки и id в пределах `recall_tokens`, побайтно стабильно для неизменённого хранилища; только `mem_pack` может вернуть абзац или тело, и он никогда не выполняется в хуке.
- Fail open: ошибка хранилища или поиска оставляет вывод хука прежним, и на пути хука сетевых вызовов не делается.
- Нет строки `Measurement` — нет заявления об экономии; счётчики промптов контрольной точки идут в состояние плагина, а не в журнал.

## Связанное

[Обзор](../overview.md) · [Контракт плагина](../plugin-contract.md) · [Ход хуков](../hook-flow.md) · [Ход MCP](../mcp-flow.md) · [Хранилище](../store.md) · [inject](inject.md) · [read](read.md)
