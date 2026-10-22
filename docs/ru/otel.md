---
lang: ru
---

# Экспорт в OpenTelemetry

rtok уже записывает каждый хук, вызов MCP и проксированный запрос в `~/.rtok/rtok.db`.
Экспортёр проецирует эти строки в OTLP/HTTP JSON и отправляет их в любой коллектор — Jaeger,
Grafana, SigNoz, Maple, — используя семантические соглашения OpenTelemetry GenAI. Это
проекция, а не второй регистратор: на пути хука ничего не выполняется, и трассу всегда можно
восстановить из базы данных (D19, `plan.md` P16; дизайн в `src/otel/PLAN.md`).

Выключен, пока не определён endpoint.

## Включение

```toml
# ~/.rtok/config.toml
[otel]
endpoint      = "http://localhost:4318"   # базовый URL OTLP/HTTP; "" = $OTEL_EXPORTER_OTLP_ENDPOINT
headers       = ""                        # "k=v,k2=v2"; "" = $OTEL_EXPORTER_OTLP_HEADERS
service_name  = "rtok"
content       = false                     # установите true, только если хотите отправлять тела, аргументы и результаты
content_bytes = 65536                     # на атрибут; сверх этого — `rtok.archive.id` → `rtok expand <id>`
flush_secs    = 5                         # интервал сброса proxy / mcp, а также таймаут POST
```

Или вообще без файла конфигурации:

```bash
export OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4318
```

Затем:

```bash
rtok otel status
```

```bash
rtok otel flush
```

`status` печатает определённый endpoint, каждую отметку (watermark), сколько строк ожидает отправки, и последнюю
ошибку экспортёра. `flush` отправляет один пакет (1 000 строк на поток) и печатает, что он отправил. Обе команды
завершаются с кодом 0 и сообщением `otel: no endpoint`, когда экспорт выключен.

## Когда происходит сброс

| Интерфейс | Триггер |
|---|---|
| `rtok proxy` | таймер каждые `flush_secs` |
| `rtok mcp` | таймер каждые `flush_secs` и ещё раз при EOF на stdin |
| хуки | `Stop` и `SessionEnd` запускают отсоединённый `rtok otel flush` и возвращаются |
| что угодно | `rtok otel flush` |

Хук никогда не открывает сокет: он передаёт работу дочернему процессу, поэтому p95 хука остаётся
ниже 10 мс при включённом экспорте. Доставка — at-least-once за отметкой на каждый поток, которая
сдвигается только при ответе 2xx; id спанов выводятся из id строк, поэтому повторно отправленный спан побайтно идентичен, и
коллекторы его объединяют. Параллельные сбрасыватели (таймер прокси, таймер mcp, дочерний процесс, запущенный хуком) берут
эксклюзивную блокировку файла рядом с БД, поэтому они передают работу друг другу, а не отправляют один и тот же пакет дважды (T16.9).

**Известный долг (T16.9):** сбросы по таймеру `proxy` и `mcp` могут пересекаться с отсоединённым `rtok otel flush`
из `Stop`/`SessionEnd`. Параллельные процессы могут состязаться за одну и ту же отметку и экспортировать пакет
дважды; тогда счётчики ожидающих строк в `rtok otel status` могут расходиться с тем, что получил коллектор.
Пока T16.9 не внедрён, при отладке экспорта предпочитайте один долгоживущий интерфейс или сбрасывайте вручную
при остановленных `rtok proxy` / `rtok mcp`.

## Что вы увидите

| Строка журнала | Спан | Ключевые атрибуты |
|---|---|---|
| сессия | `invoke_agent {host}` (корень) | `gen_ai.agent.name`, `gen_ai.conversation.id`, `rtok.project`, `rtok.cwd` |
| `PreToolUse` / `PostToolUse` | `execute_tool {tool}` | `gen_ai.tool.name`, `gen_ai.tool.call.id`, `gen_ai.tool.call.arguments`, `gen_ai.tool.call.result` |
| любой другой хук | `hook {event}` | `rtok.hook.event`; `UserPromptSubmit` добавляет `gen_ai.input.messages` |
| вызов MCP-инструмента | `execute_tool {name}` | `gen_ai.tool.name`, `rtok.plugin`, аргументы и результат |
| проксированный запрос | `chat {model}` (CLIENT) | `gen_ai.provider.name`, `gen_ai.request.model`, `gen_ai.usage.input_tokens`, `gen_ai.usage.output_tokens`, `gen_ai.usage.cache_read.input_tokens`, `gen_ai.usage.cache_creation.input_tokens`, оба тела сообщений |

Каждый спан также несёт `gen_ai.conversation.id`, `rtok.surface`, `rtok.kind` и `rtok.call.id`,
а ошибка задаёт статус спана плюс `error.type`. Запуски плагинов и экономия передаются как события
спана (`rtok.plugin.run`, `rtok.measurement` с `rtok.tokens.saved`). Строки `logs` становятся записями
логов в той же трассе. Три накопительные суммы уходят в `/v1/metrics`: `rtok.tokens`
(по `gen_ai.token.type` и модели; только оплаченное провайдером, поэтому ответ, отданный
семантическим кэшем, не учитывается), `rtok.tokens.saved` (по плагину) и `rtok.calls`.

Содержимое сообщений, аргументы и результаты инструментов по умолчанию не отправляются, даже если endpoint
настроен. Установите `content = true`, только если хотите передавать эти данные. При включённом содержимом
значения длиннее `content_bytes` обрезаются, а спан содержит id архива, поэтому полные байты доступны через
`rtok expand <id>`.

## Бэкенды

Локальным примерам с `docker run` / `docker compose` ниже нужен запущенный Docker-совместимый движок.
Предпочитайте Colima из mise (`docs/colima.md`); Docker Desktop не нужен.


### Jaeger

```bash
docker run --rm -p 16686:16686 -p 4318:4318 jaegertracing/jaeger:2.11.0
```

`endpoint = "http://localhost:4318"`, затем откройте <http://localhost:16686> и выберите сервис `rtok`.
Jaeger 2.x обслуживает только трассы: на `/v1/logs` и `/v1/metrics` он отвечает 404, что
`rtok otel flush` сообщает как `not served: logs, metrics` и пропускает. Отметка `logs` остаётся
на месте, поэтому `rtok otel status` продолжает считать ожидающие строки логов — это ожидаемо, а не ошибка.

### Grafana

```bash
docker run --rm -p 3000:3000 -p 4318:4318 grafana/otel-lgtm
```

`endpoint = "http://localhost:4318"`, затем <http://localhost:3000> (admin/admin): Explore →
Tempo для трасс, Loki для логов, Prometheus для `rtok_tokens_total` и `rtok_tokens_saved_total`.

Вместо этого Grafana Cloud:

```toml
[otel]
endpoint = "https://otlp-gateway-<region>.grafana.net/otlp"
headers  = "Authorization=Basic <base64 of instanceID:token>"
```

### SigNoz

Self-hosted, из checkout SigNoz:

```bash
docker compose -f deploy/docker/docker-compose.yaml up -d
```

`endpoint = "http://localhost:4318"`. Вместо этого SigNoz Cloud:

```toml
[otel]
endpoint = "https://ingest.<region>.signoz.cloud:443"
headers  = "signoz-ingestion-key=<key>"
```

### Maple

```toml
[otel]
endpoint = "https://api.maple.dev/otlp"   # OTLP URL вашей рабочей области
headers  = "Authorization=Bearer <key>"
```

Maple читает атрибуты GenAI напрямую, поэтому спаны `chat` показывают модель, количество токенов и
сообщения без дополнительного сопоставления.

## Устранение неполадок

- `otel: no endpoint` — не задан ни `[otel] endpoint`, ни `OTEL_EXPORTER_OTLP_ENDPOINT`.
- Ничего не появляется — запустите `rtok otel flush` вручную; сбой печатается и сохраняется, поэтому
  `rtok otel status` показывает последнюю ошибку и счётчики ожидающих строк.
- Строки продолжают накапливаться — отметка сдвигается только при ответе 2xx. Проверьте собственный лог коллектора;
  обычная причина — неверный путь (`/v1/traces` добавляется к базовому URL) или отсутствующий ключ.
- Трассы есть, а промптов нет — `content = false`, или тело было длиннее `content_bytes`.
- `not served: logs, metrics` — у бэкенда нет конвейера для этого потока (Jaeger); rtok
  пропускает его без записи в лог, поскольку записанная ошибка сама стала бы ожидающей строкой лога.
- `rtok_calls_total` есть, а `rtok_tokens_total` нет — в журнале нет строк `usage`; токены берутся
  из прокси или импортированного транскрипта, а одни лишь хуки считают только вызовы.

## Проверка данных без бэкенда

`tools/otlp_validator.py` — автономный приёмник OTLP/HTTP JSON, который заново реализует правила
кодирования из спецификации и сообщает, что показал бы бэкенд:

```bash
python3 tools/otlp_validator.py 4318
```

Направьте `endpoint` на `http://127.0.0.1:4318`, запустите `rtok otel flush`, затем остановите приёмник через Ctrl-C, чтобы
напечатать полученные им спаны, логи и метрики, а также все нарушения спецификации.
