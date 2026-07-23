# MT5 utilities

`QuantForge/QuantForgeHistoryExporterEA.mq5` is a non-trading Expert Advisor.
It exports completed OHLCV bars from the connected broker terminal in bounded
chunks and publishes the dataset only after its coverage check and metadata
export both succeed.

The exporter deliberately requires `InpBrokerTimezone`. MT5 timestamps are
broker-server wall time, while the MT5 API does not expose a durable IANA
timezone identifier. QuantForge will refuse to normalize these timestamps from
an implicit guess.

The output is written below the terminal's Common Files directory:

```text
QuantForge/<dataset>.tsv
QuantForge/<dataset>.metadata.csv
```

The metadata records broker/server identity, terminal build, symbol contract
properties, sessions, swap settings, the timezone assumption and an explicit
commission input. MT5 does not expose account commission through the symbol
information API, so `InpCommissionPerLotRoundTurn` must be verified against the
account specification before promotion-grade research.
## Indicator parity probe

`QuantForge/QuantForgeIndicatorParityProbeEA.mq5` is a non-trading
strategy-tester probe. It exports MT5 reference buffers and the exact completed
bars used to validate every indicator in QuantForge's export-safe grammar. A
sample EURUSD M15 tester configuration is supplied beside the source; its
broker, terminal build, symbol, timeframe and period are embedded in every
output row.
