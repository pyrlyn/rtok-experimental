---
lang: uk
---

# Експорт OpenTelemetry

rtok уже записує кожен хук, виклик MCP і запит через проксі в `~/.rtok/rtok.db`.
Експортер проєктує ці рядки на OTLP/HTTP JSON і надсилає їх до будь-якого колектора — Jaeger,
Grafana, SigNoz, Maple — за семантичними конвенціями OpenTelemetry GenAI. Це
проєкція, а не другий записувач: на шляху хука нічого не виконується, і трасу завжди можна
відновити з бази даних (D19, `plan.md` P16; дизайн у `src/otel/PLAN.md`).

Вимкнено, доки не визначено endpoint.

## Як увімкнути

```toml
# ~/.rtok/config.toml
[otel]
endpoint      = "http://localhost:4318"   # базова URL-адреса OTLP/HTTP; "" = $OTEL_EXPORTER_OTLP_ENDPOINT
headers       = ""                        # "k=v,k2=v2"; "" = $OTEL_EXPORTER_OTLP_HEADERS
service_name  = "rtok"
content       = false                     # встановіть true, лише якщо хочете надсилати тіла, аргументи й результати
content_bytes = 65536                     # на атрибут; понад це — `rtok.archive.id` → `rtok expand <id>`
flush_secs    = 5                         # інтервал скидання для proxy / mcp і тайм-аут POST
```

Або взагалі без файлу конфігурації:

```bash
export OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4318
```

Потім:

```bash
rtok otel status
```

```bash
rtok otel flush
```

`status` виводить визначений endpoint, кожну позначку (watermark), скільки рядків очікує і останню
помилку експортера. `flush` надсилає один пакет (1 000 рядків на потік) і виводить, що надіслав. Обидві команди
завершуються з кодом 0 і повідомленням `otel: no endpoint`, коли експорт вимкнено.

## Коли відбувається скидання

| Інтерфейс | Тригер |
|---|---|
| `rtok proxy` | таймер кожні `flush_secs` |
| `rtok mcp` | таймер кожні `flush_secs` і ще раз на EOF у stdin |
| хуки | `Stop` і `SessionEnd` запускають від'єднаний `rtok otel flush` і повертаються |
| будь-що | `rtok otel flush` |

Хук ніколи не відкриває сокет: він передає роботу дочірньому процесу, тому p95 хука лишається
нижче 10 мс з увімкненим експортом. Доставка відбувається щонайменше один раз (at-least-once) за позначкою кожного потоку, яка
просувається лише на 2xx; id спанів виводяться з id рядків, тож повторно надісланий спан побайтово ідентичний і
колектори його об'єднують. Паралельні процеси скидання (таймер proxy, таймер mcp, дочірній процес, запущений хуком) беруть
ексклюзивне файлове блокування поруч із БД, тож передають роботу один одному, а не надсилають той самий пакет двічі (T16.9).

**Відомий борг (T16.9):** скидання за таймером у `proxy` і `mcp` можуть перекриватися з від'єднаним `rtok otel flush`
від `Stop`/`SessionEnd`. Паралельні процеси можуть змагатися за ту саму позначку й експортувати
пакет двічі; кількість очікуваних рядків у `rtok otel status` тоді може не збігатися з тим, що отримав колектор.
Доки T16.9 не реалізовано, під час налагодження експорту віддавайте перевагу одному довготривалому інтерфейсу або скидайте вручну,
зупинивши `rtok proxy` / `rtok mcp`.

## Що ви бачите

| Рядок журналу обліку | Спан | Ключові атрибути |
|---|---|---|
| сесія | `invoke_agent {host}` (корінь) | `gen_ai.agent.name`, `gen_ai.conversation.id`, `rtok.project`, `rtok.cwd` |
| `PreToolUse` / `PostToolUse` | `execute_tool {tool}` | `gen_ai.tool.name`, `gen_ai.tool.call.id`, `gen_ai.tool.call.arguments`, `gen_ai.tool.call.result` |
| будь-який інший хук | `hook {event}` | `rtok.hook.event`; `UserPromptSubmit` додає `gen_ai.input.messages` |
| виклик інструмента MCP | `execute_tool {name}` | `gen_ai.tool.name`, `rtok.plugin`, аргументи й результат |
| запит через проксі | `chat {model}` (CLIENT) | `gen_ai.provider.name`, `gen_ai.request.model`, `gen_ai.usage.input_tokens`, `gen_ai.usage.output_tokens`, `gen_ai.usage.cache_read.input_tokens`, `gen_ai.usage.cache_creation.input_tokens`, обидва тіла повідомлень |

Кожен спан також несе `gen_ai.conversation.id`, `rtok.surface`, `rtok.kind` і `rtok.call.id`,
а помилка задає статус спану плюс `error.type`. Запуски плагінів і економія передаються як події
спану (`rtok.plugin.run`, `rtok.measurement` з `rtok.tokens.saved`). Рядки `logs` стають записами
логів у тій самій трасі. Три кумулятивні суми надходять до `/v1/metrics`: `rtok.tokens`
(за `gen_ai.token.type` і моделлю; лише оплачене провайдером, тому відповідь, віддана
семантичним кешем, не враховується), `rtok.tokens.saved` (за плагіном) і `rtok.calls`.

Вміст повідомлень, аргументи й результати інструментів за замовчуванням не надсилаються, навіть якщо endpoint
налаштовано. Установіть `content = true`, лише якщо хочете передавати ці дані. Коли вміст увімкнено,
значення довші за `content_bytes` обрізаються, а спан містить id архіву, тож повні байти доступні через
`rtok expand <id>`.

## Бекенди

Для локальних прикладів `docker run` / `docker compose` нижче потрібен запущений сумісний із Docker рушій.
Віддавайте перевагу Colima з mise (`docs/colima.md`); Docker Desktop не потрібен.


### Jaeger

```bash
docker run --rm -p 16686:16686 -p 4318:4318 jaegertracing/jaeger:2.11.0
```

`endpoint = "http://localhost:4318"`, потім відкрийте <http://localhost:16686> і виберіть сервіс `rtok`.
Jaeger 2.x обслуговує лише траси: на `/v1/logs` і `/v1/metrics` він відповідає 404, що
`rtok otel flush` повідомляє як `not served: logs, metrics` і пропускає. Позначка `logs` лишається
на місці, тож `rtok otel status` і далі рахує очікувані рядки логів — це очікувано, а не помилка.

### Grafana

```bash
docker run --rm -p 3000:3000 -p 4318:4318 grafana/otel-lgtm
```

`endpoint = "http://localhost:4318"`, потім <http://localhost:3000> (admin/admin): Explore →
Tempo для трас, Loki для логів, Prometheus для `rtok_tokens_total` і `rtok_tokens_saved_total`.

Або Grafana Cloud:

```toml
[otel]
endpoint = "https://otlp-gateway-<region>.grafana.net/otlp"
headers  = "Authorization=Basic <base64 of instanceID:token>"
```

### SigNoz

Самостійно розгорнутий, з checkout SigNoz:

```bash
docker compose -f deploy/docker/docker-compose.yaml up -d
```

`endpoint = "http://localhost:4318"`. Або SigNoz Cloud:

```toml
[otel]
endpoint = "https://ingest.<region>.signoz.cloud:443"
headers  = "signoz-ingestion-key=<key>"
```

### Maple

```toml
[otel]
endpoint = "https://api.maple.dev/otlp"   # OTLP URL вашого робочого простору
headers  = "Authorization=Bearer <key>"
```

Maple читає атрибути GenAI напряму, тож спани `chat` показують модель, кількість токенів і
повідомлення без додаткового зіставлення.

## Усунення несправностей

- `otel: no endpoint` — не задано ні `[otel] endpoint`, ні `OTEL_EXPORTER_OTLP_ENDPOINT`.
- Нічого не з'являється — запустіть `rtok otel flush` вручну; збій виводиться й зберігається, тож
  `rtok otel status` показує останню помилку й кількість очікуваних рядків.
- Рядки накопичуються — позначка просувається лише на 2xx. Перевірте власний лог колектора;
  звичайна причина — неправильний шлях (`/v1/traces` дописується до базової URL-адреси) або відсутній ключ.
- Траси є, а промптів немає — `content = false` або тіло було довшим за `content_bytes`.
- `not served: logs, metrics` — бекенд не має конвеєра для цього потоку (Jaeger); rtok
  пропускає його без логування, бо записана в лог помилка сама стала б очікуваним рядком логу.
- `rtok_calls_total` є, а `rtok_tokens_total` немає — у журналі обліку немає рядків `usage`; токени надходять
  із проксі або з імпортованого транскрипту, самі лише хуки рахують тільки виклики.

## Перевірка даних без бекенда

`tools/otlp_validator.py` — це автономний приймач OTLP/HTTP JSON, який заново реалізує правила
кодування зі специфікації й повідомляє, що показав би бекенд:

```bash
python3 tools/otlp_validator.py 4318
```

Спрямуйте `endpoint` на `http://127.0.0.1:4318`, запустіть `rtok otel flush`, потім зупиніть приймач через Ctrl-C, щоб
вивести отримані спани, логи й метрики, а також усі порушення специфікації.
