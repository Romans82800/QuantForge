#!/bin/sh
set -eu

if [ "$#" -ne 1 ]; then
  echo "usage: $0 EXPORT_DIRECTORY" >&2
  exit 2
fi

source_dir=$1
if [ ! -d "$source_dir" ]; then
  echo "export directory does not exist: $source_dir" >&2
  exit 2
fi

metadata_value() {
  property=$1
  file=$2
  awk -F, -v property="$property" '$1 == property { gsub(/\r/, "", $2); print $2; exit }' "$file"
}

for metadata_path in "$source_dir"/*_H1_2020_present.metadata.csv; do
  [ -f "$metadata_path" ] || continue

  symbol=$(metadata_value symbol "$metadata_path")
  broker=$(metadata_value broker "$metadata_path")
  server=$(metadata_value server "$metadata_path")
  terminal_build=$(metadata_value terminal_build "$metadata_path")
  digits=$(metadata_value digits "$metadata_path")
  point=$(metadata_value point "$metadata_path")
  tick_size=$(metadata_value tick_size "$metadata_path")
  tick_value=$(metadata_value tick_value "$metadata_path")
  contract_size=$(metadata_value contract_size "$metadata_path")
  volume_min=$(metadata_value volume_min "$metadata_path")
  volume_step=$(metadata_value volume_step "$metadata_path")
  volume_max=$(metadata_value volume_max "$metadata_path")
  stops_level=$(metadata_value stops_level_points "$metadata_path")
  freeze_level=$(metadata_value freeze_level_points "$metadata_path")
  filling_flags=$(metadata_value filling_mode_flags "$metadata_path")
  trade_mode_raw=$(metadata_value trade_mode "$metadata_path")
  margin_initial=$(metadata_value margin_initial "$metadata_path")
  swap_mode_raw=$(metadata_value swap_mode "$metadata_path")
  swap_long=$(metadata_value swap_long "$metadata_path")
  swap_short=$(metadata_value swap_short "$metadata_path")
  triple_swap_day=$(metadata_value triple_swap_day "$metadata_path" | tr '[:upper:]' '[:lower:]')
  timezone=$(metadata_value broker_timezone "$metadata_path")
  account_currency=$(metadata_value account_currency "$metadata_path")
  base_currency=$(metadata_value currency_base "$metadata_path")
  profit_currency=$(metadata_value currency_profit "$metadata_path")
  margin_currency=$(metadata_value currency_margin "$metadata_path")

  filling_modes='[]'
  if [ "$filling_flags" -eq 0 ]; then
    filling_modes='["return"]'
  else
    [ $((filling_flags & 1)) -eq 0 ] || filling_modes=$(printf '%s' "$filling_modes" | jq '. + ["fill_or_kill"]')
    [ $((filling_flags & 2)) -eq 0 ] || filling_modes=$(printf '%s' "$filling_modes" | jq '. + ["immediate_or_cancel"]')
    [ $((filling_flags & 4)) -eq 0 ] || filling_modes=$(printf '%s' "$filling_modes" | jq '. + ["book_or_cancel"]')
  fi

  case "$trade_mode_raw" in
    SYMBOL_TRADE_MODE_DISABLED) trade_mode=disabled ;;
    SYMBOL_TRADE_MODE_LONGONLY) trade_mode=long_only ;;
    SYMBOL_TRADE_MODE_SHORTONLY) trade_mode=short_only ;;
    SYMBOL_TRADE_MODE_CLOSEONLY) trade_mode=close_only ;;
    SYMBOL_TRADE_MODE_FULL) trade_mode=full ;;
    *) echo "unsupported trade mode for $symbol: $trade_mode_raw" >&2; exit 1 ;;
  esac

  case "$swap_mode_raw" in
    SYMBOL_SWAP_MODE_DISABLED) swap_mode=disabled ;;
    SYMBOL_SWAP_MODE_POINTS) swap_mode=points ;;
    SYMBOL_SWAP_MODE_CURRENCY_SYMBOL) swap_mode=symbol_currency ;;
    SYMBOL_SWAP_MODE_CURRENCY_MARGIN) swap_mode=margin_currency ;;
    SYMBOL_SWAP_MODE_CURRENCY_DEPOSIT) swap_mode=deposit_currency ;;
    SYMBOL_SWAP_MODE_CURRENCY_PROFIT) swap_mode=profit_currency ;;
    SYMBOL_SWAP_MODE_INTEREST_CURRENT) swap_mode=interest_current ;;
    SYMBOL_SWAP_MODE_INTEREST_OPEN) swap_mode=interest_open ;;
    SYMBOL_SWAP_MODE_REOPEN_CURRENT) swap_mode=reopen_current ;;
    SYMBOL_SWAP_MODE_REOPEN_BID) swap_mode=reopen_bid ;;
    *) echo "unsupported swap mode for $symbol: $swap_mode_raw" >&2; exit 1 ;;
  esac

  if awk -v value="$margin_initial" 'BEGIN { exit !(value > 0) }'; then
    margin_json=$margin_initial
  else
    margin_json=null
  fi

  sessions=$(awk -F, '
    function day_name(day) {
      if (day == 0) return "sunday"
      if (day == 1) return "monday"
      if (day == 2) return "tuesday"
      if (day == 3) return "wednesday"
      if (day == 4) return "thursday"
      if (day == 5) return "friday"
      return "saturday"
    }
    $1 ~ /^session_[0-6]_[0-9]+$/ {
      gsub(/\r/, "", $2)
      split($2, values, "|")
      split(values[2], opened, ":")
      split(values[3], closed, ":")
      printf "{\"day\":\"%s\",\"open_minute\":%d,\"close_minute\":%d}\n",
             day_name(values[1]), opened[1] * 60 + opened[2], closed[1] * 60 + closed[2]
    }
  ' "$metadata_path" | jq -s '.')

  swap_multipliers=$(jq -n \
    --argjson sunday "$(metadata_value swap_multiplier_sunday "$metadata_path" | awk '{printf "%d", $1}')" \
    --argjson monday "$(metadata_value swap_multiplier_monday "$metadata_path" | awk '{printf "%d", $1}')" \
    --argjson tuesday "$(metadata_value swap_multiplier_tuesday "$metadata_path" | awk '{printf "%d", $1}')" \
    --argjson wednesday "$(metadata_value swap_multiplier_wednesday "$metadata_path" | awk '{printf "%d", $1}')" \
    --argjson thursday "$(metadata_value swap_multiplier_thursday "$metadata_path" | awk '{printf "%d", $1}')" \
    --argjson friday "$(metadata_value swap_multiplier_friday "$metadata_path" | awk '{printf "%d", $1}')" \
    --argjson saturday "$(metadata_value swap_multiplier_saturday "$metadata_path" | awk '{printf "%d", $1}')" \
    '[
      {day:"sunday", multiplier:$sunday},
      {day:"monday", multiplier:$monday},
      {day:"tuesday", multiplier:$tuesday},
      {day:"wednesday", multiplier:$wednesday},
      {day:"thursday", multiplier:$thursday},
      {day:"friday", multiplier:$friday},
      {day:"saturday", multiplier:$saturday}
    ]')

  output_path="$source_dir/${symbol}.broker.json"
  jq -n \
    --arg profile_name "$broker · $server · $symbol · MT5 build $terminal_build" \
    --arg symbol "$symbol" \
    --argjson digits "$digits" \
    --argjson point "$point" \
    --argjson tick_size "$tick_size" \
    --argjson tick_value "$tick_value" \
    --argjson contract_size "$contract_size" \
    --argjson volume_min "$volume_min" \
    --argjson volume_step "$volume_step" \
    --argjson volume_max "$volume_max" \
    --argjson stops_level "$stops_level" \
    --argjson freeze_level "$freeze_level" \
    --argjson filling_modes "$filling_modes" \
    --arg trade_mode "$trade_mode" \
    --argjson margin_initial "$margin_json" \
    --arg swap_mode "$swap_mode" \
    --argjson swap_long "$swap_long" \
    --argjson swap_short "$swap_short" \
    --arg triple_swap_day "$triple_swap_day" \
    --argjson swap_multipliers "$swap_multipliers" \
    --argjson sessions "$sessions" \
    --arg timezone "$timezone" \
    --arg account_currency "$account_currency" \
    --arg base_currency "$base_currency" \
    --arg profit_currency "$profit_currency" \
    --arg margin_currency "$margin_currency" \
    '{
      profile_name: $profile_name,
      symbol: $symbol,
      digits: $digits,
      point: $point,
      tick_size: $tick_size,
      tick_value: $tick_value,
      contract_size: $contract_size,
      volume_min: $volume_min,
      volume_step: $volume_step,
      volume_max: $volume_max,
      stops_level_points: $stops_level,
      freeze_level_points: $freeze_level,
      filling_modes: $filling_modes,
      trade_mode: $trade_mode,
      margin_initial_per_lot: $margin_initial,
      swap_mode: $swap_mode,
      swap_long: $swap_long,
      swap_short: $swap_short,
      triple_swap_day: $triple_swap_day,
      swap_multipliers: $swap_multipliers,
      sessions: $sessions,
      timezone: $timezone,
      account_currency: $account_currency,
      base_currency: $base_currency,
      profit_currency: $profit_currency,
      margin_currency: $margin_currency,
      synthetic_spreads: []
    }' > "$output_path"

  echo "$output_path"
done
