# Strategy Negater

SQX-style side flip for robustness / cross-check. Swaps long ↔ short entry and exit trees and inverts `side` (`LongOnly` ↔ `ShortOnly`). `Both` strategies are rejected as ambiguous.

```powershell
quantforge negate --strategy path/to/strategy.ir.json --out negated.json
# then scout/judge the `strategy` field inside the report
```
