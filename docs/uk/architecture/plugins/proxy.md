---
lang: uk
---

# Плагін proxy

![Запит LLM потрапляє до rtok proxy, записується як рядок calls, за бажанням переписується фільтрами proxy_filter інших плагінів, перевіряється за семантичним кешем, пересилається вгору з потоком, що розгалужується без змін, і його usage записується в SQLite](../../../architecture/plugins/proxy.svg)

Джерело: [`proxy.d2`](../../../architecture/plugins/proxy.d2). Документація плагіна: [README](../../../../src/plugins/proxy/README.md). Індекс: [усі діаграми](../README.md).

Плагін `proxy` є стороною поверхні проксі, що захоплює використання. `rtok proxy` є локальним
HTTP-переходом, до якого хост звертається через `ANTHROPIC_BASE_URL` або `OPENAI_BASE_URL`.
Сам плагін є лише маніфестом (`Surface::Proxy`, `default_on: true`); реалізацією є сервер у
`src/proxy/`. Він записує кожен запит і реальний `usage` провайдера, а в режимі `compress` є
етапом, на якому методи `proxy_filter` інших плагінів скорочують запит, перш ніж він піде назовні.

## Покроково

1. **Надходить запит.** Резервний обробник axum `proxy` викликає `handle`. Він визначає смугу
   (`lane::classify`), відхиляє запит, який уже пройшов `MAX_HOPS` переходів rtok, зі статусом
   508, і займає слот `max_in_flight` смуги, якщо його налаштовано.
2. **Звичайний режим.** `ProxyState::plain()` істинний, коли `[proxy] enabled`, `[core] enabled`
   або `[plugins.proxy] enabled` має значення false, а також для запиту, який уже сформував
   попередній перехід rtok. Звичайний запит пересилається побайтово і з'являється лише в кільцевому
   буфері `/live` (`live::push`).
3. **Формування запиту.** `shape_request` працює в блокуючому потоці. `route::apply` може
   замінити модель для смуги, яка не є агентською, потім `record` оновлює сесію і вставляє рядок
   `calls`.
4. **Стиснення.** Коли `[proxy] mode = "compress"` і політика смуги це дозволяє, `compress`
   відкриває `Runtime`, обходить `Registry::enabled()` і викликає `proxy_filter` кожного плагіна
   для `WireRequest` (зараз `archive`, `json_tree`, `toon`, `compress`). `archive` пропускається,
   коли контекстні правки платформи увімкнено, а `toon` — коли політика смуги його вимикає. Кожне
   повернене `Measurement` проходить через `cx.record`. Якщо нічого не змінилося, назовні
   йдуть оригінальні байти.
5. **Формування для провайдера.** Далі йдуть `noise::strip` (лише в режимі compress),
   `prepare` (`stream_options` для OpenAI), `context_edits`, `rewrite_tools`
   (`[proxy.tools_rewrite]`) і `flex::apply`, кожен за власним перемикачем. `rewrite_tools`
   архівує оригінальний `tools[]` і записує його id у `Measurement.ref_id`.
6. **Семантичний кеш.** Коли `[plugins.proxy.semantic_cache] enabled` увімкнено і смуга це
   дозволяє, `semantic_cache::build_prompt` і `Cache::lookup` можуть відповісти з пам'яті.
   `cache_response` тоді записує рядок `semantic_cache_hit` і usage, декодований із збереженого
   тіла, і жодного звернення до вищого рівня не відбувається.
7. **Пересилання і розгалуження.** `flex::send` надсилає запит до смуги, дротової або стандартної
   вищої ланки. Породжене завдання копіює кожен фрагмент клієнту без змін (SSE ніколи не
   буферизується для клієнта), одночасно тримаючи обмежену копію для облікових записів.
8. **Запис відповіді.** `finish` встановлює `calls.ms`, записує `call_io`, наповнює кеш повними
   тілами і вставляє рядок `usage` (а також рядок `tokens` провайдера, коли блок usage розібрано
   через `wire::usage_from_response`).
9. **Оператори читають.** `rtok stats`, `rtok tui` і `rtok web` читають ці рядки; `GET /health`
   і `GET /live` обслуговує `src/proxy/cli.rs`.

## Де це живе

| Блок | Код |
| --- | --- |
| rtok proxy, охоронці | `src/proxy/mod.rs` (`app`, `handle`, `hops_of`, `MAX_HOPS`) |
| Звичайний режим? | `src/proxy/mod.rs` (`ProxyState::plain`) |
| route, record | `src/proxy/route.rs` (`apply`), `src/proxy/mod.rs` (`shape_request`, `record`) |
| compress | `src/proxy/mod.rs` (`compress`); `Plugin::proxy_filter` у `crates/rtok-plugin-sdk/` |
| Плагіни proxy_filter | `src/plugins/archive/`, `src/plugins/json_tree/`, `src/plugins/toon/`, `src/plugins/compress/` |
| Формування для провайдера | `src/proxy/noise.rs`, `src/proxy/anthropic.rs` (`apply_context_edits`), `src/proxy/tools_rewrite.rs`, `src/proxy/flex.rs` |
| Адаптери дроту | `src/proxy/wire.rs`, `anthropic.rs`, `openai_chat.rs`, `openai_responses.rs`, `gemini.rs` |
| Семантичний кеш | `src/proxy/semantic_cache.rs`, `src/proxy/mod.rs` (`cache_response`) |
| Розгалуження, finish | `src/proxy/mod.rs` (`handle`, `finish`, `record_usage`) |
| Маніфест | `src/plugins/proxy/mod.rs` (`ProxyPlugin`) |
| Конфігурація | `crates/rtok-config/` (`[proxy]`, `[proxy.lanes.<lane>]`, `[plugins.proxy]`) |
| Таблиці | `crates/rtok-store/` (`calls`, `call_io`, `usage`, `tokens`, `measurements`) |

## Інваріанти

- Відмовостійкість: тіло, яке не розбирається, аварійно завершене завдання формування, помилка
  сховища або незмінений результат пересилають оригінальні байти, а запит ніколи не відхиляється
  через облік.
- Звичайний режим не змінює жодного байта; `plugins.proxy.enabled = false` також зупиняє рядки
  `calls`, `call_io` і `usage`.
- Кожен сформований запит вставляє один рядок `usage`; помилкова або без блоку usage відповідь
  все одно має рядок з нулями.
- Лосслес: переписування плагіном спершу архівує оригінал, і `expand <id>` його відновлює.
- Маршрутизований виклик, виклик керування контекстом і влучання в кеш записують рядки, які не
  заявляють власної економії токенів; економією є лише рядки, які записали плагіни.
- Слухач прив'язується до `[proxy] bind` (за замовчуванням `127.0.0.1`), а `accept-encoding`
  клієнта замінюється на `identity`, щоб розгалуження могло прочитати тіло.

## Пов'язане

[Огляд](../overview.md) · [Контракт плагіна](../plugin-contract.md) · [Потік проксі](../proxy-flow.md) ·
[archive](archive.md) · [toon](toon.md) · [json_tree](json_tree.md) · [compress](compress.md)
