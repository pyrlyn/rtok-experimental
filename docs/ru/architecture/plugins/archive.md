---
lang: ru
---

# Плагин archive

![Запрос попадает в archive через фильтр прокси или через контекст pi; старые крупные результаты инструментов архивируются, заменяются указателем и воспроизводятся байт в байт, а expand возвращает оригинал](../../../architecture/plugins/archive.svg)

Источник: [`archive.d2`](../../../architecture/plugins/archive.d2). Документация плагина: [README](../../../../src/plugins/archive/README.md). Оглавление: [все диаграммы](../README.md).

`archive` сокращает результаты инструментов, которые вышли за пределы живой зоны: результаты старше `keep_turns` ходов и больше `min_tokens`. Оригинал уходит в хранилище архива, а блок, который видит модель, становится строкой-указателем плюс первые и последние строки оригинала. Решение сохраняется для каждого `tool_use_id`, поэтому каждый последующий запрос воспроизводит те же байты, и кэш промптов провайдера остаётся действительным.

## Пошагово

1. **Носитель передаёт запрос.** При `proxy.mode = "compress"` прокси запускает `Plugin::proxy_filter` каждого включённого плагина; `ArchivePlugin::proxy_filter` сразу возвращает управление, если режим не `compress`. Без прокси pi вызывает событие `context`, и расширение запускает `rtok archive rewrite --stdin`, который вызывает `archive::pi::rewrite_stdin`. Оба носителя используют одни и те же функции `rewrite` и `rewrite_blobs`; `pi::tool_results` и `pi::live_blobs` лишь приводят массив сообщений pi к нормализованному для провода виду (`ToolResultRef`, `BlobRef`).
2. **Выбор полос.** `rewrite(req.tool_results(), cx)` выполняется всегда. `rewrite_blobs` запускается только когда `[plugins.archive] live_blobs` равно true, а `rewrite_skills` — только когда `skills` равно true; оба выключены по умолчанию.
3. **Остаться за пределами живой зоны.** `outside_live_zone` отбрасывает каждый результат, у которого `turn` меньше `keep_turns`. `toon` вызывает ту же функцию, так что граница определяется один раз.
4. **Посмотреть ранее принятое решение.** `rewrite_block` вызывает `cx.archive_decision(tool_use_id)`. Раскрытый id или строка, указатель которой начинается с префикса `toon` или `json_tree`, не трогается (эти плагины разделяют `archive_decisions` и уже измерили свою экономию). Любой другой сохранённый указатель воспроизводится как записан и измеряется заново.
5. **Проверить размер.** Пока решения нет, `cx.estimate(text, Class::Code)` ниже `min_tokens` оставляет блок без изменений и ничего не записывает.
6. **Архивировать и записать указатель.** `cx.put_archive` сохраняет исходные байты. `pointer()` формирует `[archived <id8>: N lines · T tokens · expand(<id8>)]`, за которым следуют `head_lines` и `tail_lines` оригинала, каждая строка обрезана через `clip`. При `tiers = true` `tier_live` выводит одну строку указателя (L0) и добавляет извлечение без потерь (L1) для результатов ровно на ходе `keep_turns`.
7. **Сохранить решение.** `cx.put_archive_decision` сохраняет текст указателя до пересылки запроса. Ошибка хранилища на любом шаге оставляет блок без изменений.
8. **Измерить.** Каждый переписанный блок даёт `Measurement` (`plugin: "archive"`, вид `pointer`, `tier_l0`, `tier_l1`, `live_blob` или `skill`). `record_run` группирует их под одним вызовом `plugin_run` с оценками токенов `before` и `after`.
9. **Раскрыть.** Модель вызывает MCP-инструмент `expand` с 8-символьным id. `expand::fetch_range` разрешает префикс, читает оригинал из архива, выставляет `expanded_ts` и записывает измерение `expand` в одной транзакции. Начиная со следующего запроса `rewrite_block` видит раскрытое решение и оставляет исходный текст без изменений.

## Где это находится

| Блок | Код |
| --- | --- |
| `ArchivePlugin::proxy_filter`, проверка режима | `src/plugins/archive/mod.rs` (`ArchivePlugin`, `proxy_filter`) |
| Носитель pi | `src/plugins/archive/pi.rs` (`rewrite_stdin`, `tool_results`, `live_blobs`); `src/cli/archive.rs` |
| Полосы | `src/plugins/archive/mod.rs` (`rewrite`, `rewrite_blobs`, `rewrite_skills`) |
| Граница живой зоны | `src/plugins/archive/mod.rs` (`outside_live_zone`) |
| Решение, проверка размера | `src/plugins/archive/mod.rs` (`rewrite_block`, `rewrite_blob`, `rewrite_skill`) |
| Текст указателя | `src/plugins/archive/mod.rs` (`pointer`, `pointer_line`, `clip`, `tier_live`, `l1_extract`) |
| Измерение и строка запуска | `src/plugins/archive/mod.rs` (`record_run`) |
| `expand` | `src/expand.rs` (`fetch_range`, `resolve_id`); `src/mcp.rs` (`expand_text`) |
| Таблицы | `crates/rtok-store/src/schema.rs` (`archive`, `archive_decisions`, `measurements`) |
| Конфигурация | `crates/rtok-config/src/lib.rs` (секция `Archive`: `keep_turns`, `min_tokens`, `head_lines`, `tail_lines`, `tiers`, `live_blobs`, `skills`) |

## Инварианты

- Без потерь: оригинал сохраняется до того, как блок меняется, и `expand <id>` его возвращает.
- Детерминированность: текст указателя сохраняется для каждого `tool_use_id` (для блобов — для каждого хеша содержимого), поэтому повтор побайтно идентичен и префикс промпта не сдвигается.
- Никогда не трогаются: `system`, `tools`, последние `keep_turns` ходов, раскрытый id и части-изображения или иные не текстовые части (`block_text` возвращает `None` для них).
- Fail open: любая ошибка хранилища оставляет блок прежним.
- Одна `Measurement` на переписанный блок; нет строки — нет заявления об экономии.
- `live_blobs`, `skills` и `tiers` включаются явно и по умолчанию равны false.

## Связанное

[Обзор](../overview.md) · [Контракт плагина](../plugin-contract.md) · [proxy](proxy.md) · [toon](toon.md) · [compress](compress.md) · [cmd](cmd.md)
