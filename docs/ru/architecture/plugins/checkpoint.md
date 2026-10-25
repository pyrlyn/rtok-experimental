---
lang: ru
---

# Плагин checkpoint

![PreCompact и SessionEnd читают транскрипт и сохраняют заметку checkpoint; SessionStart читает заметку обратно как инъекцию с бюджетом](../../../architecture/plugins/checkpoint.svg)

Источник: [`checkpoint.d2`](../../../architecture/plugins/checkpoint.d2). Исходный код модуля: [`checkpoint.rs`](../../../../src/plugins/checkpoint.rs). Оглавление: [все диаграммы](../README.md).

`checkpoint` не является плагином из каталога: у него нет манифеста и таблицы `[plugins.checkpoint]`. Это модуль, который вызывают `inject` и ветка `SessionEnd` диспетчера хуков. Перед тем как хост сожмёт контекст или когда сессия заканчивается, он сжимает JSONL-транскрипт в короткую заметку: последние набранные запросы, затронутые файлы, последние ошибки и id архива, которые ещё живы. После сжатия или при следующем запуске заметка читается обратно как одна инъекция, чтобы модель не начинала вслепую.

## Пошагово

1. **Сохранить по PreCompact.** Диспетчер хуков вызывает `pre_compact` у каждого включённого плагина; `inject` вызывает `checkpoint::save(transcript_path, cx)`, который записывает заметку вида `checkpoint:<session>`.
2. **Сохранить по SessionEnd.** Ветка `SessionEnd` в `src/hooks/mod.rs` вызывает `checkpoint::save_session` внутри `catch_unwind` и игнорирует результат. Она использует тот же экстрактор и рендер, вид `session:<session>`, и относит заметку к имени проекта.
3. **Извлечь.** `extract_path` читает транскрипт построчно, поэтому большая сессия никогда не держится в памяти целиком. `worth_parsing` пропускает строки, которые не могут содержать путь, ошибку, набранный запрос или тело навыка; тест сравнивает отфильтрованный и неотфильтрованный прогоны (`prefilter_is_a_true_superset_on_fixtures_and_a_large_transcript`). Результат — `Checkpoint`: последние 20 набранных запросов, пути файлов с `Action` (`Edited`, `Created`, `Deleted`, `Read`; сначала изменённые файлы), последние 8 строк ошибок (пропущенных через `redact_private`) и тела навыков, которые внедрил хост.
4. **Прикрепить id архива.** `attach_ids` читает `session_live_archives` и добавляет id, начиная с самых новых, пока `render()` ещё помещается в `[plugins.memory] checkpoint_tokens`; id, который не влезает, отбрасывается, и цикл останавливается.
5. **Сохранить.** `write` вызывает `upsert_note(project, kind, "compact", render())`, поэтому повторный PreCompact или SessionEnd одной сессии заменяет заметку, а не добавляет новую. Он также сохраняет счётчики набранных и пропущенных запросов как строку состояния плагина (ключ — значение); эти счётчики — сигнал качества, а не `Measurement`.
6. **Восстановить после сжатия.** При `SessionStart` с `source = "compact"` `inject::session_start` вызывает `checkpoint::offer`, который читает последнюю заметку этой сессии.
7. **Восстановить при запуске.** При `source = "startup"`, включённых `[plugins.memory] enabled` и `startup_recall`, `inject` вызывает `checkpoint::offer_session`: самую новую заметку `session:*` проекта (с откатом к старому имени, привязанному к checkout), а также `Measurement` вида `handoff` у плагина `memory`.
8. **Ограничить инъекцию бюджетом.** `render_offer` снова запускает `redact_private` (старые заметки могут содержать приватный тег), обрезает текст через `fit_budget` до `checkpoint_tokens` и возвращает `Injection` с `priority` 9. `inject` помещает её в `additionalContext` хука.
9. **Побочное использование.** `last_paths(cx, root)` возвращает файлы последнего checkpoint; карта репозитория графа и код файлов памяти используют её, чтобы персонализировать свой вывод.

## Где это находится

| Блок | Код |
| --- | --- |
| Подключение PreCompact / SessionStart | `src/plugins/inject/mod.rs` (`pre_compact`, `session_start`) |
| Подключение SessionEnd | `src/hooks/mod.rs` (ветка `"SessionEnd"`) |
| `save`, `save_session`, `write` | `src/plugins/checkpoint.rs` |
| `extract_path`, `worth_parsing` | `src/plugins/checkpoint.rs` (`extract_lines_with`) |
| `Checkpoint`, `Action`, `PathEntry` | `src/plugins/checkpoint.rs` |
| `attach_ids` | `src/plugins/checkpoint.rs` (`attach_ids`, `checkpoint_cap`) |
| `offer`, `offer_session`, `render_offer` | `src/plugins/checkpoint.rs` |
| `last_paths` | `src/plugins/checkpoint.rs`; вызывающие в `src/plugins/graph/mod.rs` и `src/plugins/memory/files.rs` |
| Таблицы | `crates/rtok-store/src/schema.rs` (`notes`, `archive_decisions`, `measurements`) |
| Конфигурация | `crates/rtok-config/src/lib.rs` (`Memory`: `checkpoint_tokens`, `startup_recall`) |

## Инварианты

- Вызывающие хуки никогда не падают из-за него: PreCompact игнорирует результат `save`, а SessionEnd оборачивает `save_session` в `catch_unwind`.
- Ограниченное состояние: экстрактор хранит только последние запросы и ошибки и набор путей, поэтому память не растёт вместе с транскриптом.
- Приватный текст редактируется до сохранения и ещё раз перед восстановлением.
- Восстановление ограничено `checkpoint_tokens`; один и тот же транскрипт рендерится в побайтно одинаковый текст, и тест проверяет, что Claude, pi и OpenCode восстанавливают одно и то же тело (`pi_and_opencode_compact_restore_bytes_match_claude`).
- Единственное измерение — строка `handoff` при запуске; нет строки — нет заявления об экономии.

## Связанное

[Обзор](../overview.md) · [Контракт плагина](../plugin-contract.md) · [inject](inject.md) · [memory](memory.md) · [graph](graph.md)
