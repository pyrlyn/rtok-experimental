---
lang: ru
---

# Плагин proxy

![Запрос LLM попадает в rtok proxy, записывается строкой calls, при необходимости переписывается фильтрами proxy_filter других плагинов, проверяется по семантическому кэшу, пересылается апстриму с потоком, который разветвляется без изменений, а его usage записывается в SQLite](../../../architecture/plugins/proxy.svg)

Источник: [`proxy.d2`](../../../architecture/plugins/proxy.d2). Документация плагина: [README](../../../../src/plugins/proxy/README.md). Оглавление: [все диаграммы](../README.md).

Плагин `proxy` — это сторона прокси, отвечающая за сбор usage. `rtok proxy` — локальный HTTP-переход, на который хост заходит через `ANTHROPIC_BASE_URL` или `OPENAI_BASE_URL`. Сам плагин — лишь манифест (`Surface::Proxy`, `default_on: true`); реализация — сервер в `src/proxy/`. Он записывает каждый запрос и настоящий `usage` провайдера, а в режиме `compress` это та стадия, на которой методы `proxy_filter` других плагинов сокращают запрос перед тем, как он уйдёт.

## Пошагово

1. **Приходит запрос.** Резервный маршрут axum `proxy` вызывает `handle`. Он классифицирует полосу (`lane::classify`), отклоняет запрос, который уже прошёл `MAX_HOPS` переходов rtok, со статусом 508, и занимает слот `max_in_flight` полосы, если он настроен.
2. **Обычный режим.** `ProxyState::plain()` истинно, когда `[proxy] enabled`, `[core] enabled` или `[plugins.proxy] enabled` равно false; запрос, который уже сформировал более ранний переход rtok, обрабатывается так же. Обычный запрос пересылается побайтно и появляется только в кольцевом буфере `/live` (`live::push`).
3. **Сформировать запрос.** `shape_request` работает в блокирующем потоке. `route::apply` может подменить модель на полосе, не являющейся агентской, затем `record` обновляет сессию и вставляет строку `calls`.
4. **Сжатие.** Когда `[proxy] mode = "compress"` и политика полосы это допускает, `compress` открывает `Runtime`, проходит `Registry::enabled()` и вызывает `proxy_filter` каждого плагина над `WireRequest` (сейчас `archive`, `json_tree`, `toon`, `compress`). `archive` пропускается, когда включены правки контекста платформы, а `toon` — когда политика полосы его выключает. Каждое возвращённое `Measurement` проходит через `cx.record`. Если ничего не изменилось, уходят исходные байты.
5. **Формирование под провайдера.** Далее идут `noise::strip` (только в режиме compress), `prepare` (`stream_options` OpenAI), `context_edits`, `rewrite_tools` (`[proxy.tools_rewrite]`) и `flex::apply`, каждый за своим переключателем. `rewrite_tools` архивирует исходный `tools[]` и записывает его id в `Measurement.ref_id`.
6. **Семантический кэш.** С `[plugins.proxy.semantic_cache] enabled` и полосой, которая это допускает, `semantic_cache::build_prompt` и `Cache::lookup` могут ответить из памяти. `cache_response` тогда записывает строку `semantic_cache_hit` и usage, декодированный из сохранённого тела, и вызов апстрима не происходит.
7. **Пересылка и разветвление.** `flex::send` отправляет запрос на полосу, в провод или на апстрим по умолчанию. Порождённая задача копирует каждый фрагмент клиенту без изменений (SSE никогда не буферизуется для клиента), сохраняя ограниченную копию для учёта.
8. **Запись ответа.** `finish` выставляет `calls.ms`, записывает `call_io`, заполняет кэш полными телами и вставляет строку `usage` (а также строку провайдера `tokens`, когда блок usage разобран через `wire::usage_from_response`).
9. **Операторы читают.** `rtok stats`, `rtok tui` и `rtok web` читают эти строки; `GET /health` и `GET /live` обслуживаются `src/proxy/cli.rs`.

## Где это находится

| Блок | Код |
| --- | --- |
| rtok proxy, проверки | `src/proxy/mod.rs` (`app`, `handle`, `hops_of`, `MAX_HOPS`) |
| Обычный режим? | `src/proxy/mod.rs` (`ProxyState::plain`) |
| Маршрут, запись | `src/proxy/route.rs` (`apply`), `src/proxy/mod.rs` (`shape_request`, `record`) |
| compress | `src/proxy/mod.rs` (`compress`); `Plugin::proxy_filter` в `crates/rtok-plugin-sdk/` |
| Плагины proxy_filter | `src/plugins/archive/`, `src/plugins/json_tree/`, `src/plugins/toon/`, `src/plugins/compress/` |
| Формирование под провайдера | `src/proxy/noise.rs`, `src/proxy/anthropic.rs` (`apply_context_edits`), `src/proxy/tools_rewrite.rs`, `src/proxy/flex.rs` |
| Адаптеры провода | `src/proxy/wire.rs`, `anthropic.rs`, `openai_chat.rs`, `openai_responses.rs`, `gemini.rs` |
| Семантический кэш | `src/proxy/semantic_cache.rs`, `src/proxy/mod.rs` (`cache_response`) |
| Разветвление, finish | `src/proxy/mod.rs` (`handle`, `finish`, `record_usage`) |
| Манифест | `src/plugins/proxy/mod.rs` (`ProxyPlugin`) |
| Конфигурация | `crates/rtok-config/` (`[proxy]`, `[proxy.lanes.<lane>]`, `[plugins.proxy]`) |
| Таблицы | `crates/rtok-store/` (`calls`, `call_io`, `usage`, `tokens`, `measurements`) |

## Инварианты

- Fail open: нераспознанное тело, упавшая задача формирования, ошибка хранилища или результат без изменений пересылают исходные байты, и запрос никогда не отклоняется из-за учёта.
- Обычный режим не меняет ни одного байта; `plugins.proxy.enabled = false` также останавливает записи `calls`, `call_io` и `usage`.
- Каждый сформированный запрос вставляет одну строку `usage`; ошибка или ответ без usage всё равно записывает строку с нулями.
- Без потерь: переписывание плагином сначала архивирует оригинал, и `expand <id>` его восстанавливает.
- Маршрутизированный вызов, вызов управления контекстом и попадание в кэш записывают строки, которые не заявляют собственной экономии токенов; экономию дают только строки, записанные плагинами.
- Слушатель привязан к `[proxy] bind` (по умолчанию `127.0.0.1`), а `accept-encoding` клиента заменяется на `identity`, чтобы разветвитель мог прочитать тело.

## Связанное

[Обзор](../overview.md) · [Контракт плагина](../plugin-contract.md) · [Ход прокси](../proxy-flow.md) · [archive](archive.md) · [toon](toon.md) · [json_tree](json_tree.md) · [compress](compress.md)
