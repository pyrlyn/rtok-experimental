---
lang: ru
---

# Плагин docs

![Вызов MCP разрешает крейт по Cargo.lock и ищет в локально закэшированном индексе rustdoc; только rtok docs fetch обращается к docs.rs и заполняет кэш](../../../architecture/plugins/docs.svg)

Источник: [`docs.d2`](../../../architecture/plugins/docs.d2). Документация плагина: [README](../../../../src/plugins/docs/README.md). Оглавление: [все диаграммы](../README.md).

`docs` даёт агенту документацию именно той версии крейта, с которой собирается проект, без вставки целых страниц в разговор. Три инструмента MCP разрешают зависимость из `Cargo.lock`, возвращают ранжированные фрагменты из локально закэшированного индекса rustdoc с docs.rs и по запросу загружают тело одного элемента. Плагин выключен по умолчанию (`[plugins.docs] enabled = false`) и не имеет методов хуков.

## Пошагово

1. **Приходит вызов инструмента.** Агент вызывает `docs_resolve`, `docs_query` или `docs_get` в `rtok mcp`. `src/mcp.rs` направляет каждое имя в соответствующую функцию в `src/plugins/docs/mod.rs`.
2. **Подхватить заметки рабочего пространства.** `docs_resolve` и `docs_query` сначала выполняют `ingest_llms`: файл `llms-full.txt` или `llms.txt` в рабочей директории делится по заголовкам `#` на псевдокрейт `llms` и относится к версии `local:<cwd>`, чтобы рабочие пространства не видели тексты друг друга. Неизменённые байты ничего не трогают; отсутствующий или пустой файл сбрасывает кэш этого рабочего пространства.
3. **Разрешить.** `resolve_lock` читает `Cargo.lock` из рабочей директории крейтом `toml` (без подпроцесса `cargo`) и возвращает первый пакет с этим именем. Имя и версия должны пройти `safe_part`, потому что оба попадают в URL и путь кэша. `docs_resolve` отвечает `{name, version, cached}`, где `cached` берётся из `doc_crate_cached`.
4. **Промах без ошибки.** Имя, которого нет в `Cargo.lock`, даёт `not in Cargo.lock: <name>`; версия крейта, которой нет в хранилище, даёт `not cached: rtok docs fetch <name>`. Оба — обычные однострочные результаты.
5. **Запрос.** Для закэшированной версии `docs_query` вызывает `search_doc_items`, то есть FTS5 `MATCH` по `doc_items_fts`, ограниченный `query_limit` попаданиями с `snippet_chars` на фрагмент. Затем он по одному оставляет фрагменты, пока оценка токенов JSON укладывается в `max_tokens`. Сокет не открывается.
6. **Измерить.** Когда возвращён хотя бы один фрагмент, `docs_query` записывает `Measurement` (`plugin: "docs"`, вид `query`, `ref_id` `<name>@<version>`), у которого сторона «до» — полный текст каждого учтённого попадания.
7. **Загрузить один элемент.** `docs_get(id)` возвращает путь, вид и полный текст документации одного закэшированного элемента из `doc_items` либо падает с `unknown doc id`.
8. **Загрузить только по запросу.** `rtok docs fetch <name>` вызывает `run_fetch`, который определяет версию по `Cargo.lock`, делает GET на `https://docs.rs/crate/<name>/<version>/json.zst` через `http_get`, декодирует поток zstd, а `index_rustdoc` оставляет каждый элемент, у которого есть строка документации. Сжатый файл и его sha256 записываются в `<home>/docs/<name>/<version>/`, а `replace_doc_crate` заполняет `doc_crates` и `doc_items`; триггер базы данных синхронизирует `doc_items_fts`.

## Где это находится

| Блок | Код |
| --- | --- |
| Маршрутизация MCP | `src/mcp.rs` (`invoke`, `docs_resolve`, `docs_query`, `docs_get`) |
| Определения инструментов | `src/plugins/docs/mod.rs` (`DocsPlugin::mcp_tools`) |
| `ingest_llms` | `src/plugins/docs/mod.rs` (`ingest_llms`, `split_llms`, `local_version`) |
| `resolve_lock` | `src/plugins/docs/mod.rs` (`resolve_lock`, `safe_part`) |
| Запрос и бюджет | `src/plugins/docs/mod.rs` (`docs_query`); `crates/rtok-store/src/docs.rs` (`search_doc_items`) |
| `docs_get` | `src/plugins/docs/mod.rs` (`docs_get`); `crates/rtok-store/src/docs.rs` (`get_doc_item`) |
| Загрузка | `src/cli/docs.rs`; `src/plugins/docs/mod.rs` (`run_fetch`, `fetch_crate`, `http_get`, `index_rustdoc`) |
| Таблицы | `crates/rtok-store/src/schema.rs` (`doc_crates`, `doc_items`); миграция `0039_docs_fts` (`doc_items_fts`) |
| Конфигурация | `crates/rtok-config/src/lib.rs` (`Docs`: `enabled`, `query_limit`, `snippet_chars`, `max_tokens`) |

## Инварианты

- По умолчанию выключен и не имеет метода хука: SessionStart, PreCompact и отправка запроса не затрагиваются.
- `docs_resolve`, `docs_query` и `docs_get` никогда не открывают сокет; единственный сетевой путь — `rtok docs fetch`.
- Имя или версия крейта, не безопасные для пути, никогда не используются для построения URL или пути к файлу.
- Неизвестный крейт — однострочный промах, а не завершение с ошибкой.
- Запрос, который возвращает фрагменты, записывает `docs`/`query`; без строки нет заявления об экономии.
- SQL хранилища находится в `crates/rtok-store/src/docs.rs`; таблица FTS5 — виртуальная таблица, которую DSL Diesel не умеет описывать.

## Связанное

[Обзор](../overview.md) · [Контракт плагина](../plugin-contract.md) · [Ход MCP](../mcp-flow.md) · [read](read.md) · [memory](memory.md)
