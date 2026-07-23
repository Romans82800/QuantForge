# Portfolio packing protocol

`quantforge portfolio` creates an immutable allocation proposal from an intact,
promotion-grade MAP-Elites databank. It does not alter discovery evidence or
substitute portfolio diversification for the databank's clone and correlation
gates.

## Hard constraints

The packer verifies the databank manifest, broker binding, elite structural
fingerprints, stored discovery gates and coverage map before considering an
allocation. A feasible result must satisfy every configured cap:

- maximum pairwise equity-return correlation;
- maximum weight per strategy;
- maximum aggregate weight per symbol;
- maximum aggregate weight per strategy family;
- maximum number of strategies;
- minimum expected portfolio return.

Version 1 uses equal weights. This makes exposure and weight enforcement
transparent: for a target size of `N`, each strategy receives `1/N`. QuantForge
searches feasible target sizes between the minimum implied by the weight cap and
the configured strategy maximum. Candidates are ordered deterministically by
the selected objective, then admitted only when their correlation and exposure
counts remain feasible. If no subset satisfies every hard constraint, the
command fails and writes nothing.

The available objectives are `risk-adjusted-return`, `cvar`, and
`minimize-drawdown`. The drawdown objective still obeys the configured minimum
return floor.

## Equity and stress record

Discovery stores a bounded chronological equity-delta signature for every
elite. Portfolio packing rescales each signature to its recorded total return,
combines the selected paths at their assigned weights, and records the resulting
path drawdown. Every candidate in one databank shares the same data, balance and
evaluation configuration.

Stress uses a seeded circular moving-block bootstrap on the portfolio return
path. The artifact retains every trial's return and maximum drawdown plus:

- fifth-percentile return;
- lower-tail conditional value at risk (CVaR);
- ninety-fifth-percentile maximum drawdown.

Fixed candidates, configuration and seed produce an identical report and
portfolio ID.

## Current boundary

The CLI adapter currently packs one MAP-Elites databank and therefore one
broker-bound symbol. The engine and report support multiple symbol labels, but
combining several databanks or reading Certified Vault entries is a later input
adapter. Portfolio output is a research allocation artifact, not a live order
management system or profitability claim.
