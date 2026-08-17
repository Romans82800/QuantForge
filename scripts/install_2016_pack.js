const fs = require("fs");
const path = require("path");

const staging = path.join(
  process.env.USERPROFILE,
  "Documents",
  "QuantForge",
  "tick_staging_2016",
);
const pack = path.join(
  process.env.USERPROFILE,
  "Documents",
  "QuantForge",
  "ICMarkets_EST7_2016_present",
);
const prefix = "ICMarketsSC-Demo_";
const suffix = "_2016_present";

fs.mkdirSync(pack, { recursive: true });

function csvToTsv(src, dst) {
  return new Promise((resolve, reject) => {
    const rs = fs.createReadStream(src, { encoding: "utf8" });
    const ws = fs.createWriteStream(dst, { encoding: "utf8" });
    rs.on("error", reject);
    ws.on("error", reject);
    ws.on("finish", resolve);
    rs.on("data", (chunk) => ws.write(String(chunk).replace(/,/g, "\t")));
    rs.on("end", () => ws.end());
  });
}

function stampSymbol(metaPath, symbol) {
  const text = fs.readFileSync(metaPath, "utf8");
  const out = text
    .split(/\r?\n/)
    .map((line) => (line.startsWith("symbol,") ? `symbol,${symbol}` : line))
    .join("\n");
  fs.writeFileSync(metaPath, out.endsWith("\n") ? out : `${out}\n`);
}

function metaPathForTsv(tsvPath) {
  return tsvPath.slice(0, -4) + ".metadata.csv";
}

(async () => {
  const m1s = fs
    .readdirSync(staging)
    .filter((n) => n.endsWith("_M1.csv") && !n.includes("quotes"));
  for (const name of m1s.sort()) {
    const symbol = name.slice(0, -"_M1.csv".length);
    console.log("installing", symbol);
    const m1Tsv = path.join(pack, `${prefix}${symbol}_M1${suffix}.tsv`);
    const h1Tsv = path.join(pack, `${prefix}${symbol}_H1${suffix}.tsv`);
    await csvToTsv(path.join(staging, `${symbol}_M1.csv`), m1Tsv);
    await csvToTsv(path.join(staging, `${symbol}_H1.csv`), h1Tsv);
    const m1Meta = metaPathForTsv(m1Tsv);
    const h1Meta = metaPathForTsv(h1Tsv);
    fs.copyFileSync(path.join(staging, `${symbol}_M1.metadata.csv`), m1Meta);
    fs.copyFileSync(path.join(staging, `${symbol}_H1.metadata.csv`), h1Meta);
    stampSymbol(m1Meta, symbol);
    stampSymbol(h1Meta, symbol);
    const quotes = path.join(staging, `${symbol}_M1.quotes.csv`);
    if (fs.existsSync(quotes)) {
      fs.copyFileSync(
        quotes,
        path.join(pack, `${prefix}${symbol}_M1${suffix}.quotes.csv`),
      );
      const qm = path.join(staging, `${symbol}_M1.quotes.metadata.csv`);
      if (fs.existsSync(qm)) {
        fs.copyFileSync(
          qm,
          path.join(pack, `${prefix}${symbol}_M1${suffix}.quotes.metadata.csv`),
        );
      }
      console.log("  + quotes");
    }
    console.log("  m1", fs.statSync(m1Tsv).size, "h1", fs.statSync(h1Tsv).size);
  }
  console.log("done");
})().catch((error) => {
  console.error(error);
  process.exit(1);
});
