---
lang: ru
---

# Контракт плагина

![Контракт плагина: признаки времени компиляции и конфигурация выбирают плагины в Registry; каждый плагин реализует трейт Plugin из rtok-plugin-sdk и обращается к хранилищу только через Ctx и трейты возможностей, которые реализует рантайм хоста Runtime](../../architecture/plugin-contract.svg)

Исходник: [`plugin-contract.d2`](../../architecture/plugin-contract.d2). Индекс: [все диаграммы](README.md).

Каждый метод сокращения токенов в rtok — это плагин за одним трейтом. Трейт и каждый тип, который называет плагин, живут
в `crates/rtok-plugin-sdk` (D25), поэтому плагин, встроенный в дерево или нет, зависит от контракта и никогда от бинарника
rtok. Сторона хоста, `Runtime` в `src/plugin.rs`, реализует трейты возможностей, которые требует контракт.

## Пошагово

1. **Выбор во время компиляции.** Каждый id плагина — это функция Cargo (`default = all`), а `plugins::all()` добавляет каждый
   под `#[cfg(feature = "<id>")]` в порядке диспетчеризации.
2. **Выбор во время выполнения.** `Registry::new(&config)` сопоставляет каждый скомпилированный плагин с
   `config.plugin_enabled(id, default_on)`: `[plugins.<id>] enabled`, если задано, иначе `default_on` из манифеста.
   `config::CATALOGUE` перечисляет те же пары `(id, default_on)`, и тест падает, если они расходятся, так что плагин не может
   быть зарегистрирован наполовину.
3. **Трейт.** `manifest()` и `dashboard_page()` обязательны. У каждого метода события есть пустая реализация по умолчанию,
   поэтому плагин пишет только то, что объявляет его `Manifest::surfaces`: `pre_tool` может вернуть `Deny` или `Rewrite`
   для вызова; `post_tool` возвращает дополнительный контекст и больше ничего; `session_start`, `prompt_submit` и
   `subagent_start` возвращают `Injection` (текст и приоритет), который `inject` ограничивает бюджетом; `pre_compact`
   выполняется до компакции; `mcp_tools` объявляет инструменты MCP; `proxy_filter` редактирует `WireRequest` и возвращает
   `Measurement`.
4. **События — это заимствованные представления** над разобранным входом хука (`PreToolUse<'a>`, …), поэтому путь хука
   ничего не копирует.
5. **`Ctx` — это весь мир плагина.** Он несёт id агента и типизированную конфигурацию плагина (`plugin_config::<T>(id)`) и
   разыменовывается в `&dyn Capabilities`: `Host` (сессия, cwd, `estimate`, `record`, `log`, конфигурация), `Archive`
   (хранит оригиналы для `expand`), `Notes` (память), `ReadCache`, `Ledger` и `Symbols` (индекс графа).
6. **Экономия — это `Measurement`.** `Host::record` — единственный способ заявить экономию: плагин, вид, байты и оценочные
   токены до и после, `ref_id`, который разрешает `expand`, и владеющий `call_id`.
7. **Две реализации хоста.** `Runtime` реализует каждую возможность поверх `rtok-store`; `testing::MemoryHost` реализует их
   в памяти для модульных тестов плагинов.
8. **Вне дерева.** Крейт, который встраивает rtok, передаёт свои плагины в `Registry::from_plugins`; с фичей `wasm-host`
   `.wasm`-гостевые модули добавляются в реестр во время выполнения.

## Где это находится

| Блок | Код |
| --- | --- |
| Трейт, события, `Manifest`, `Measurement`, `Injection` | `crates/rtok-plugin-sdk/src/lib.rs` |
| `Ctx` и трейты возможностей | `crates/rtok-plugin-sdk/src/host.rs` |
| `WireRequest` для `proxy_filter` | `crates/rtok-plugin-sdk/src/wire.rs` |
| Хост в памяти | `crates/rtok-plugin-sdk/src/testing.rs` (`MemoryHost`) |
| Рантайм хоста | `src/plugin.rs` (`Runtime`, `impl Host`, `impl Archive`, …) |
| Реестр | `src/plugins/mod.rs` (`all`, `Registry`) |
| Каталог | `crates/rtok-config/` (`CATALOGUE`) |
| Руководство по созданию плагинов | `docs/plugin-authoring.md`, `examples/hello_plugin.rs` |

## Инварианты

- Паника плагина перехватывается поверхностью и отбрасывает только вывод этого плагина.
- PostToolUse возвращает только контекст; у типа нет способа заменить результат инструмента.
- Ни один плагин не зависит от другого: `cargo build --no-default-features --features measure` собирается.
- Отсутствие строки `Measurement` означает отсутствие заявления об экономии, в коде, документации и отчётах.

## Связанное

[Обзор](overview.md) · [Поток хука](hook-flow.md) · [Хранилище](store.md) ·
[`docs/plugin-authoring.md`](../plugin-authoring.md) · [wasm](plugins/wasm.md)
