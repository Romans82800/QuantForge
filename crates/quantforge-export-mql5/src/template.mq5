#property strict
#property version   "1.00"
#property copyright "QuantForge. Generated strategy implementation."
#property description "Native QuantForge EA generated from typed Strategy IR."

#include <Trade/Trade.mqh>

input bool   InpAllowLiveTrading=@@ALLOW_LIVE@@;
input ulong  InpMagic=@@MAGIC@@;
input int    InpDeviationPoints=@@DEVIATION@@;
input double InpMaxSpreadPoints=@@MAX_SPREAD@@;
input double InpEstimatedSlippagePointsPerSide=@@SLIPPAGE@@;
input double InpCommissionPerLotRoundTurn=@@COMMISSION@@;
input int    InpEntryWindowStartHour=@@ENTRY_WINDOW_START@@;
input int    InpEntryWindowEndHour=@@ENTRY_WINDOW_END@@;
input string InpParityPrefix="@@PARITY_PREFIX@@";

CTrade g_trade;
datetime g_last_bar=0;
datetime g_last_exit_bar=0;
int g_decision_bars_seen=0;
int g_entry_day_key=0;
int g_entries_today=0;
int g_deals_file=INVALID_HANDLE;
int g_equity_file=INVALID_HANDLE;
int g_metadata_file=INVALID_HANDLE;
int g_quotes_file=INVALID_HANDLE;
datetime g_quote_minute=0;
bool g_quote_bucket_open=false;
double g_bid_open=0.0,g_bid_high=0.0,g_bid_low=0.0,g_bid_close=0.0;
double g_ask_open=0.0,g_ask_high=0.0,g_ask_low=0.0,g_ask_close=0.0;
long g_quote_tick_count=0;
double g_initial_volume=0.0;
double g_initial_risk=0.0;
datetime g_entry_decision_bar=0;
int g_position_decision_bars=0;
double g_peak_favorable=EMPTY_VALUE;
bool g_partial_done[];

// Strategy fingerprint: @@STRATEGY_FINGERPRINT@@
// Broker specification hash: @@BROKER_FINGERPRINT@@
// Execution policy hash: @@EXECUTION_POLICY_FINGERPRINT@@

bool QFValid(const double value)
{
   return value!=EMPTY_VALUE && MathIsValidNumber(value);
}

double QFPrice(const int field,const int shift)
{
   if(shift<0 || Bars(_Symbol,_Period)<=shift)
      return EMPTY_VALUE;
   if(field==0) return iOpen(_Symbol,_Period,shift);
   if(field==1) return iHigh(_Symbol,_Period,shift);
   if(field==2) return iLow(_Symbol,_Period,shift);
   return iClose(_Symbol,_Period,shift);
}

// Handles live for the whole run. Creating and releasing one per condition atom
// makes MT5 rebuild the indicator's history on every evaluation, which is by far
// the largest cost of a generated expert in the Strategy Tester.
#define QFH_MAX_HANDLES 48

string g_qfh_keys[QFH_MAX_HANDLES];
int    g_qfh_values[QFH_MAX_HANDLES];
int    g_qfh_count=0;

int QFHCached(const string key)
{
   for(int index=0;index<g_qfh_count;index++)
      if(g_qfh_keys[index]==key)
         return g_qfh_values[index];
   return -2;
}

int QFHRemember(const string key,const int handle)
{
   if(g_qfh_count<QFH_MAX_HANDLES)
   {
      g_qfh_keys[g_qfh_count]=key;
      g_qfh_values[g_qfh_count]=handle;
      g_qfh_count++;
   }
   return handle;
}

void QFHReleaseHandles()
{
   for(int index=0;index<g_qfh_count;index++)
      if(g_qfh_values[index]!=INVALID_HANDLE)
         IndicatorRelease(g_qfh_values[index]);
   g_qfh_count=0;
}

int QFHMa(const ENUM_MA_METHOD method,const ENUM_APPLIED_PRICE source,const int period)
{
   const string key="MA|"+IntegerToString(period)+"|"+IntegerToString((int)method)
                    +"|"+IntegerToString((int)source);
   const int cached=QFHCached(key);
   if(cached!=-2)
      return cached;
   return QFHRemember(key,iMA(_Symbol,_Period,period,0,method,source));
}

int QFHRsi(const ENUM_APPLIED_PRICE source,const int period)
{
   const string key="RSI|"+IntegerToString(period)+"|"+IntegerToString((int)source);
   const int cached=QFHCached(key);
   if(cached!=-2)
      return cached;
   return QFHRemember(key,iRSI(_Symbol,_Period,period,source));
}

int QFHAtr(const int period)
{
   const string key="ATR|"+IntegerToString(period);
   const int cached=QFHCached(key);
   if(cached!=-2)
      return cached;
   return QFHRemember(key,iATR(_Symbol,_Period,period));
}

int QFHAdx(const int period)
{
   const string key="ADXW|"+IntegerToString(period);
   const int cached=QFHCached(key);
   if(cached!=-2)
      return cached;
   return QFHRemember(key,iADXWilder(_Symbol,_Period,period));
}

int QFHStdDev(const ENUM_APPLIED_PRICE source,const int period)
{
   const string key="STDDEV|"+IntegerToString(period)+"|"+IntegerToString((int)source);
   const int cached=QFHCached(key);
   if(cached!=-2)
      return cached;
   return QFHRemember(key,iStdDev(_Symbol,_Period,period,0,MODE_SMA,source));
}

double QFBufferValue(const int handle,const int shift)
{
   if(handle==INVALID_HANDLE || shift<0)
      return EMPTY_VALUE;
   double values[1];
   const int copied=CopyBuffer(handle,0,shift,1,values);
   if(copied!=1)
      return EMPTY_VALUE;
   return values[0];
}

double QFBufferValueAt(const int handle,const int buffer,const int shift)
{
   if(handle==INVALID_HANDLE || buffer<0 || shift<0)
      return EMPTY_VALUE;
   double values[1];
   const int copied=CopyBuffer(handle,buffer,shift,1,values);
   if(copied!=1)
      return EMPTY_VALUE;
   return values[0];
}

double QFMA(const ENUM_MA_METHOD method,const ENUM_APPLIED_PRICE source,
            const int period,const int shift)
{
   return QFBufferValue(QFHMa(method,source,period),shift);
}

double QFRSI(const ENUM_APPLIED_PRICE source,const int period,const int shift)
{
   return QFBufferValue(QFHRsi(source,period),shift);
}

double QFATR(const int period,const int shift)
{
   return QFBufferValue(QFHAtr(period),shift);
}

double QFADX(const int period,const int shift)
{
   // QuantForge's typed ADX/+DI/-DI implementation uses Welles Wilder
   // smoothing. MT5 exposes that exact buffer family separately from iADX.
   return QFBufferValue(QFHAdx(period),shift);
}

double QFPlusDI(const int period,const int shift)
{
   return QFBufferValueAt(QFHAdx(period),1,shift);
}

double QFMinusDI(const int period,const int shift)
{
   return QFBufferValueAt(QFHAdx(period),2,shift);
}

double QFStdDev(const ENUM_APPLIED_PRICE source,const int period,const int shift)
{
   return QFBufferValue(QFHStdDev(source,period),shift);
}

double QFExtreme(const ENUM_SERIESMODE mode,const int field,const int period,const int shift,
                 const bool maximum)
{
   const int index=maximum
                   ? iHighest(_Symbol,_Period,mode,period,shift)
                   : iLowest(_Symbol,_Period,mode,period,shift);
   if(index<0)
      return EMPTY_VALUE;
   return QFPrice(field,index);
}

double QFZScore(const int field,const ENUM_APPLIED_PRICE source,
                const int period,const int shift)
{
   const double value=QFPrice(field,shift);
   const double mean=QFMA(MODE_SMA,source,period,shift);
   const double deviation=QFStdDev(source,period,shift);
   if(!QFValid(value) || !QFValid(mean) || !QFValid(deviation) || deviation<=0.0)
      return EMPTY_VALUE;
   return (value-mean)/deviation;
}

double QFPercentile(const int field,const ENUM_SERIESMODE mode,
                    const int period,const int shift)
{
   const double value=QFPrice(field,shift);
   const double low=QFExtreme(mode,field,period,shift,false);
   const double high=QFExtreme(mode,field,period,shift,true);
   if(!QFValid(value) || !QFValid(low) || !QFValid(high) || high<=low)
      return EMPTY_VALUE;
   return (value-low)/(high-low)*100.0;
}

double QFROC(const int field,const int period,const int shift)
{
   const double current=QFPrice(field,shift);
   const double previous=QFPrice(field,shift+period);
   if(!QFValid(current) || !QFValid(previous) || previous==0.0)
      return EMPTY_VALUE;
   return (current/previous-1.0)*100.0;
}

double QFBodyRangeRatio(const int shift)
{
   const double open=QFPrice(0,shift);
   const double high=QFPrice(1,shift);
   const double low=QFPrice(2,shift);
   const double close=QFPrice(3,shift);
   if(!QFValid(open) || !QFValid(high) || !QFValid(low) || !QFValid(close))
      return EMPTY_VALUE;
   const double range=high-low;
   if(range<=0.0)
      return EMPTY_VALUE;
   return MathAbs(close-open)/range;
}

double QFCloseLocationInBar(const int shift)
{
   const double high=QFPrice(1,shift);
   const double low=QFPrice(2,shift);
   const double close=QFPrice(3,shift);
   if(!QFValid(high) || !QFValid(low) || !QFValid(close))
      return EMPTY_VALUE;
   const double range=high-low;
   if(range<=0.0)
      return EMPTY_VALUE;
   return (close-low)/range;
}

double QFAtrPercentile(const int atr_period,const int lookback,const int shift)
{
   if(atr_period<1 || lookback<1 || shift<0 || Bars(_Symbol,_Period)<shift+lookback)
      return EMPTY_VALUE;
   // One bulk copy instead of `lookback` single-value reads. CopyBuffer returns
   // oldest-first, so index `copied-1-offset` is the bar at `shift+offset`.
   double window[];
   const int copied=CopyBuffer(QFHAtr(atr_period),0,shift,lookback,window);
   if(copied<1)
      return EMPTY_VALUE;
   double values[];
   ArrayResize(values,copied);
   int finite=0;
   double current=EMPTY_VALUE;
   for(int offset=0;offset<copied;offset++)
   {
      const double value=window[copied-1-offset];
      if(!QFValid(value))
         continue;
      values[finite++]=value;
      if(offset==0)
         current=value;
   }
   if(finite==0 || !QFValid(current))
      return EMPTY_VALUE;
   int rank=0;
   for(int index=0;index<finite;index++)
      if(values[index]<=current)
         rank++;
   return (double)rank/(double)finite*100.0;
}

bool QFSameBrokerDay(const datetime left,const datetime right)
{
   MqlDateTime a,b;
   if(!TimeToStruct(left,a) || !TimeToStruct(right,b))
      return false;
   return a.year==b.year && a.mon==b.mon && a.day==b.day;
}

double QFSessionRangeExtreme(const int start_hour,const int range_bars,const int shift,
                             const bool want_high)
{
   // Mirror quantforge_sqx::session_range_series (see sqx_template.mq5).
   if(range_bars<1 || shift<0 || Bars(_Symbol,_Period)<=shift)
      return EMPTY_VALUE;
   const datetime anchor=iTime(_Symbol,_Period,shift);
   if(anchor<=0)
      return EMPTY_VALUE;
   MqlDateTime parts;
   if(!TimeToStruct(anchor,parts))
      return EMPTY_VALUE;

   int window_start=-1;
   for(int index=Bars(_Symbol,_Period)-1;index>=shift;index--)
   {
      const datetime bar_time=iTime(_Symbol,_Period,index);
      if(bar_time<=0 || !QFSameBrokerDay(bar_time,anchor))
         continue;
      MqlDateTime bar_parts;
      if(!TimeToStruct(bar_time,bar_parts))
         continue;
      if(bar_parts.hour>=start_hour)
      {
         window_start=index;
         break;
      }
   }
   if(window_start<0)
      return EMPTY_VALUE;

   const int window_end=window_start-(range_bars-1);
   if(window_end<shift)
      return EMPTY_VALUE;

   double extreme=want_high ? -DBL_MAX : DBL_MAX;
   int collected=0;
   for(int index=window_start;index>=window_end;index--)
   {
      const datetime bar_time=iTime(_Symbol,_Period,index);
      if(bar_time<=0 || !QFSameBrokerDay(bar_time,anchor))
         return EMPTY_VALUE;
      MqlDateTime bar_parts;
      if(!TimeToStruct(bar_time,bar_parts) || bar_parts.hour<start_hour)
         return EMPTY_VALUE;
      if(want_high)
         extreme=MathMax(extreme,iHigh(_Symbol,_Period,index));
      else
         extreme=MathMin(extreme,iLow(_Symbol,_Period,index));
      collected++;
   }
   if(collected!=range_bars)
      return EMPTY_VALUE;
   return extreme;
}

double QFSessionRangeHigh(const int start_hour,const int range_bars,const int shift)
{
   return QFSessionRangeExtreme(start_hour,range_bars,shift,true);
}

double QFSessionRangeLow(const int start_hour,const int range_bars,const int shift)
{
   return QFSessionRangeExtreme(start_hour,range_bars,shift,false);
}

double QFSwingBaseZoneExtreme(const int swing_left,const int swing_right,const int base_bars,
                              const int shift,const bool zone_high)
{
   // Match Rust `swing_base_zone_series`: forward carry of the most recent
   // confirmed pivot zone up to the evaluation bar (no future pivots).
   // Ready delay is max(swing_right, base_bars) so base_bars > swing_right
   // still forms a zone (required for short-side reclaim genes).
   if(swing_left<1 || swing_right<1 || base_bars<1)
      return EMPTY_VALUE;
   const int total=Bars(_Symbol,_Period);
   const int ready_delay=(swing_right>base_bars ? swing_right : base_bars);
   if(total<=shift+ready_delay+swing_left)
      return EMPTY_VALUE;
   const int target_ri=total-1-shift;
   double last_zone=EMPTY_VALUE;
   for(int ri=0;ri<=target_ri;ri++)
   {
      if(ri<ready_delay)
         continue;
      const int pivot_ri=ri-ready_delay;
      if(pivot_ri<swing_left)
         continue;
      const int pivot_shift=total-1-pivot_ri;
      bool is_swing_low=true;
      bool is_swing_high=true;
      for(int offset=1;offset<=swing_left;offset++)
      {
         if(iLow(_Symbol,_Period,pivot_shift)>iLow(_Symbol,_Period,pivot_shift+offset))
            is_swing_low=false;
         if(iHigh(_Symbol,_Period,pivot_shift)<iHigh(_Symbol,_Period,pivot_shift+offset))
            is_swing_high=false;
      }
      for(int offset=1;offset<=swing_right;offset++)
      {
         if(iLow(_Symbol,_Period,pivot_shift)>=iLow(_Symbol,_Period,pivot_shift-offset))
            is_swing_low=false;
         if(iHigh(_Symbol,_Period,pivot_shift)<=iHigh(_Symbol,_Period,pivot_shift-offset))
            is_swing_high=false;
      }
      const bool use_pivot=zone_high ? is_swing_low : is_swing_high;
      if(!use_pivot)
         continue;
      const int base_start_ri=pivot_ri+1;
      const int base_end_ri=base_start_ri+base_bars-1;
      if(base_end_ri>ri)
         continue;
      double extreme=zone_high ? -DBL_MAX : DBL_MAX;
      for(int bi=base_start_ri;bi<=base_end_ri;bi++)
      {
         const int bar_shift=total-1-bi;
         if(zone_high)
            extreme=MathMax(extreme,iHigh(_Symbol,_Period,bar_shift));
         else
            extreme=MathMin(extreme,iLow(_Symbol,_Period,bar_shift));
      }
      last_zone=extreme;
   }
   return last_zone;
}

double QFSwingBaseZoneHigh(const int swing_left,const int swing_right,const int base_bars,
                           const int shift)
{
   return QFSwingBaseZoneExtreme(swing_left,swing_right,base_bars,shift,true);
}

double QFSwingBaseZoneLow(const int swing_left,const int swing_right,const int base_bars,
                          const int shift)
{
   return QFSwingBaseZoneExtreme(swing_left,swing_right,base_bars,shift,false);
}

double QFLiquiditySweepScore(const int period,const int shift)
{
   if(period<1 || Bars(_Symbol,_Period)<shift+period+1)
      return 0.0;
   double prior_high=-DBL_MAX;
   double prior_low=DBL_MAX;
   for(int offset=1;offset<=period;offset++)
   {
      prior_high=MathMax(prior_high,iHigh(_Symbol,_Period,shift+offset));
      prior_low=MathMin(prior_low,iLow(_Symbol,_Period,shift+offset));
   }
   const double high=iHigh(_Symbol,_Period,shift);
   const double low=iLow(_Symbol,_Period,shift);
   const double close=iClose(_Symbol,_Period,shift);
   if(low<prior_low && close>prior_low)
      return 1.0;
   if(high>prior_high && close<prior_high)
      return -1.0;
   return 0.0;
}

double QFAverageRange(const int period,const int shift)
{
   if(period<1 || Bars(_Symbol,_Period)<shift+period)
      return EMPTY_VALUE;
   double sum=0.0;
   for(int index=shift;index<shift+period;index++)
      sum+=iHigh(_Symbol,_Period,index)-iLow(_Symbol,_Period,index);
   return sum/(double)period;
}

double QFContext(const int field,const int shift)
{
   const datetime value=iTime(_Symbol,_Period,shift);
   if(value<=0)
      return EMPTY_VALUE;
   MqlDateTime parts;
   if(!TimeToStruct(value,parts))
      return EMPTY_VALUE;
   return field==0 ? (double)parts.hour : (double)parts.day_of_week;
}

@@QF_EXTENDED_INDICATORS@@

bool QFGreater(const double left,const double right)
{
   return QFValid(left) && QFValid(right) && left>right;
}

bool QFLess(const double left,const double right)
{
   return QFValid(left) && QFValid(right) && left<right;
}

bool QFBetween(const double value,const double lower,const double upper)
{
   return QFValid(value) && QFValid(lower) && QFValid(upper)
          && value>=lower && value<=upper;
}

bool QFCrossAbove(const double current_left,const double current_right,
                  const double previous_left,const double previous_right)
{
   return QFValid(current_left) && QFValid(current_right)
          && QFValid(previous_left) && QFValid(previous_right)
          && current_left>current_right && previous_left<=previous_right;
}

bool QFCrossBelow(const double current_left,const double current_right,
                  const double previous_left,const double previous_right)
{
   return QFValid(current_left) && QFValid(current_right)
          && QFValid(previous_left) && QFValid(previous_right)
          && current_left<current_right && previous_left>=previous_right;
}

bool QFLongSignal(const int extra_shift)
{
   return @@LONG_SIGNAL@@;
}

bool QFShortSignal(const int extra_shift)
{
   return @@SHORT_SIGNAL@@;
}

bool QFExitSignal(const int extra_shift)
{
   if(!QFOwnPosition())
      return false;
   const long position_type = PositionGetInteger(POSITION_TYPE);
   if(position_type == POSITION_TYPE_BUY)
      return @@LONG_EXIT_SIGNAL@@;
   if(position_type == POSITION_TYPE_SELL)
      return @@SHORT_EXIT_SIGNAL@@;
   return false;
}

bool QFFilters(const int extra_shift)
{
   return @@FILTERS@@;
}

double QFStopDistance()
{
   return @@STOP_DISTANCE@@;
}

double QFTargetDistance(const double stop_distance)
{
   return @@TARGET_DISTANCE@@;
}

double QFRiskBudget()
{
   return @@RISK_BUDGET@@;
}

int QFEntryOrderKind()
{
   return @@ENTRY_ORDER_KIND@@;
}

double QFEntryDistance()
{
   return @@ENTRY_DISTANCE@@;
}

int QFEntryExpiryBars()
{
   return @@ENTRY_EXPIRY@@;
}

double QFBreakEvenAtR()
{
   return @@BREAK_EVEN_R@@;
}

int QFTrailingKind()
{
   return @@TRAILING_KIND@@;
}

double QFTrailingActivateR()
{
   return @@TRAILING_ACTIVATE_R@@;
}

double QFTrailingDistance()
{
   return @@TRAILING_DISTANCE@@;
}

bool QFFlattenEndOfDay()
{
   return @@FLATTEN_EOD@@;
}

bool QFMaxOneEntryPerDay()
{
   return @@MAX_ONE_ENTRY_PER_DAY@@;
}

int QFBrokerDayKey(const datetime bar_time)
{
   MqlDateTime current;
   TimeToStruct(bar_time,current);
   return current.year*10000+current.mon*100+current.day;
}

bool QFInMandatoryEntryWindow(const datetime bar_time)
{
   // QuantForge entry session, start inclusive and end exclusive, in
   // broker/chart local time. Must match the window the backtest was run with.
   MqlDateTime current;
   TimeToStruct(bar_time,current);
   return current.hour>=InpEntryWindowStartHour && current.hour<InpEntryWindowEndHour;
}

void QFSyncEntryDay(const datetime bar_time)
{
   const int day_key=QFBrokerDayKey(bar_time);
   if(day_key!=g_entry_day_key)
   {
      g_entry_day_key=day_key;
      g_entries_today=0;
   }
}

bool QFEntryDayExhausted(const datetime bar_time)
{
   if(!QFMaxOneEntryPerDay())
      return false;
   QFSyncEntryDay(bar_time);
   return g_entries_today>=1;
}

void QFMarkEntrySignalTaken(const datetime bar_time)
{
   QFSyncEntryDay(bar_time);
   g_entries_today=1;
}

int QFPartialCount()
{
   return @@PARTIAL_COUNT@@;
}

double QFPartialAtR(const int index)
{
   @@PARTIAL_AT_R@@
}

double QFPartialFraction(const int index)
{
   @@PARTIAL_FRACTION@@
}

double QFNormalizeVolume(const double requested)
{
   const double minimum=SymbolInfoDouble(_Symbol,SYMBOL_VOLUME_MIN);
   const double maximum=SymbolInfoDouble(_Symbol,SYMBOL_VOLUME_MAX);
   const double step=SymbolInfoDouble(_Symbol,SYMBOL_VOLUME_STEP);
   if(requested<=0.0 || step<=0.0)
      return 0.0;
   double volume=MathFloor(requested/step+1.0e-12)*step;
   volume=MathMin(volume,maximum);
   if(volume+1.0e-12<minimum)
      return 0.0;
   int digits=0;
   double scaled=step;
   while(digits<8 && MathAbs(scaled-MathRound(scaled))>1.0e-10)
   {
      scaled*=10.0;
      digits++;
   }
   return NormalizeDouble(volume,digits);
}

bool QFOpenOrder(const bool buy)
{
   MqlTick tick;
   if(!SymbolInfoTick(_Symbol,tick))
      return false;
   const double stop_distance=QFStopDistance();
   const double target_distance=QFTargetDistance(stop_distance);
   if(!QFValid(stop_distance) || !QFValid(target_distance)
      || stop_distance<=0.0 || target_distance<=0.0)
      return false;
   const int entry_kind=QFEntryOrderKind();
   const double entry_distance=QFEntryDistance();
   if(entry_kind!=0 && (!QFValid(entry_distance) || entry_distance<=0.0))
      return false;
   const double minimum_distance=(double)SymbolInfoInteger(_Symbol,SYMBOL_TRADE_STOPS_LEVEL)*_Point;
   if(stop_distance<minimum_distance || target_distance<minimum_distance
      || (entry_kind!=0 && entry_distance<minimum_distance))
      return false;

   const double reference=buy ? tick.ask : tick.bid;
   double intended_entry=reference;
   if(entry_kind==1)
      intended_entry=buy ? reference+entry_distance : reference-entry_distance;
   else if(entry_kind==2)
      intended_entry=buy ? reference-entry_distance : reference+entry_distance;
   intended_entry=NormalizeDouble(intended_entry,_Digits);
   const double stop=NormalizeDouble(buy ? intended_entry-stop_distance
                                         : intended_entry+stop_distance,_Digits);
   const double target=NormalizeDouble(buy ? intended_entry+target_distance
                                           : intended_entry-target_distance,_Digits);
   // Keep risk sizing identical to the QuantForge evaluator by using the
   // bound broker geometry.  Runtime OrderCalcProfit can differ between
   // terminals (especially indices/crypto), so it remains only a fallback
   // when an exported profile is incomplete.
   const double bound_tick_size=@@BROKER_TICK_SIZE@@;
   const double bound_tick_value=@@BROKER_TICK_VALUE@@;
   const double tick_size=bound_tick_size;
   const double tick_value=bound_tick_value;
   double loss_per_lot=stop_distance/tick_size*tick_value;
   if(tick_size<=0.0 || tick_value<=0.0 || !MathIsValidNumber(loss_per_lot))
   {
      loss_per_lot=0.0;
      const ENUM_ORDER_TYPE market_type=buy ? ORDER_TYPE_BUY : ORDER_TYPE_SELL;
      if(!OrderCalcProfit(market_type,_Symbol,1.0,intended_entry,stop,loss_per_lot))
         return false;
      loss_per_lot=MathAbs(loss_per_lot);
   }
   if(tick_size<=0.0 || tick_value<=0.0)
      return false;
   const double slippage_cost=2.0*InpEstimatedSlippagePointsPerSide*_Point/tick_size*tick_value;
   const double risk_per_lot=loss_per_lot+InpCommissionPerLotRoundTurn+slippage_cost;
   if(risk_per_lot<=0.0)
      return false;
   const double volume=QFNormalizeVolume(QFRiskBudget()/risk_per_lot);
   if(volume<=0.0)
      return false;

   g_trade.SetExpertMagicNumber(InpMagic);
   g_trade.SetDeviationInPoints(InpDeviationPoints);
   g_trade.SetTypeFillingBySymbol(_Symbol);
   const string comment="QF-@@FINGERPRINT_SHORT@@";
   bool sent=false;
   if(entry_kind==0)
      sent=buy
           ? g_trade.Buy(volume,_Symbol,0.0,stop,target,comment)
           : g_trade.Sell(volume,_Symbol,0.0,stop,target,comment);
   else
   {
      const datetime expiration=iTime(_Symbol,_Period,0)
                                +(datetime)(QFEntryExpiryBars()*PeriodSeconds(_Period));
      if(entry_kind==1)
         sent=buy
              ? g_trade.BuyStop(volume,intended_entry,_Symbol,stop,target,
                                ORDER_TIME_SPECIFIED,expiration,comment)
              : g_trade.SellStop(volume,intended_entry,_Symbol,stop,target,
                                 ORDER_TIME_SPECIFIED,expiration,comment);
      else
         sent=buy
              ? g_trade.BuyLimit(volume,intended_entry,_Symbol,stop,target,
                                 ORDER_TIME_SPECIFIED,expiration,comment)
              : g_trade.SellLimit(volume,intended_entry,_Symbol,stop,target,
                                  ORDER_TIME_SPECIFIED,expiration,comment);
   }
   if(!sent)
      Print("QuantForge entry rejected: ",g_trade.ResultRetcodeDescription());
   else if(entry_kind==0)
      g_initial_volume=0.0;
   return sent;
}

bool QFOwnPosition()
{
   if(!PositionSelect(_Symbol))
      return false;
   return (ulong)PositionGetInteger(POSITION_MAGIC)==InpMagic;
}

bool QFOwnPendingOrder()
{
   for(int index=OrdersTotal()-1;index>=0;index--)
   {
      const ulong ticket=OrderGetTicket(index);
      if(ticket==0)
         continue;
      if(OrderGetString(ORDER_SYMBOL)==_Symbol
         && (ulong)OrderGetInteger(ORDER_MAGIC)==InpMagic)
         return true;
   }
   return false;
}

void QFCancelOwnOrders()
{
   for(int index=OrdersTotal()-1;index>=0;index--)
   {
      const ulong ticket=OrderGetTicket(index);
      if(ticket==0)
         continue;
      if(OrderGetString(ORDER_SYMBOL)==_Symbol
         && (ulong)OrderGetInteger(ORDER_MAGIC)==InpMagic
         && !g_trade.OrderDelete(ticket))
         Print("QuantForge pending-order cancellation rejected: ",
               g_trade.ResultRetcodeDescription());
   }
}

void QFResetPositionState()
{
   g_initial_volume=0.0;
   g_initial_risk=0.0;
   g_entry_decision_bar=0;
   g_position_decision_bars=0;
   g_peak_favorable=EMPTY_VALUE;
   ArrayInitialize(g_partial_done,false);
}

// Fill-aware favorable extreme from completed decision bar's M1 path.
// Entry minute: close only. Later minutes: high/low. Ignores pre-entry minutes.
double QFFavorableSampleSinceEntry(const bool buy)
{
   if(!QFOwnPosition())
      return EMPTY_VALUE;
   const datetime entry=(datetime)PositionGetInteger(POSITION_TIME);
   const int entry_shift=iBarShift(_Symbol,PERIOD_M1,entry,false);
   if(entry_shift<0)
      return EMPTY_VALUE;
   const datetime entry_minute=iTime(_Symbol,PERIOD_M1,entry_shift);
   const datetime completed_start=iTime(_Symbol,_Period,1);
   const datetime completed_end=iTime(_Symbol,_Period,0);
   if(completed_start<=0 || completed_end<=completed_start)
      return EMPTY_VALUE;
   MqlRates rates[];
   const int copied=CopyRates(_Symbol,PERIOD_M1,completed_start,completed_end-1,rates);
   if(copied<=0)
      return EMPTY_VALUE;
   const double completed_spread=(double)iSpread(_Symbol,_Period,1)*_Point;
   double best=EMPTY_VALUE;
   for(int index=0;index<copied;index++)
   {
      if(rates[index].time<entry_minute)
         continue;
      double sample;
      if(rates[index].time==entry_minute)
         sample=rates[index].close;
      else
         sample=buy ? rates[index].high : rates[index].low;
      if(!buy)
         sample+=completed_spread;
      if(best==EMPTY_VALUE)
         best=sample;
      else
         best=buy ? MathMax(best,sample) : MathMin(best,sample);
   }
   return best;
}

void QFRatchetPeakFavorable(const bool buy,const double sample)
{
   if(!QFValid(sample))
      return;
   if(g_peak_favorable==EMPTY_VALUE)
      g_peak_favorable=sample;
   else
      g_peak_favorable=buy ? MathMax(g_peak_favorable,sample)
                           : MathMin(g_peak_favorable,sample);
}

void QFCapturePositionState()
{
   if(!QFOwnPosition() || g_initial_volume>0.0)
      return;
   const double entry=PositionGetDouble(POSITION_PRICE_OPEN);
   const double stop=PositionGetDouble(POSITION_SL);
   g_initial_volume=PositionGetDouble(POSITION_VOLUME);
   g_initial_risk=MathAbs(entry-stop);
   g_entry_decision_bar=iTime(_Symbol,_Period,0);
   g_position_decision_bars=0;
   ArrayInitialize(g_partial_done,false);
}

double QFNormalizePartialVolume(const double requested,const double remaining)
{
   const double minimum=SymbolInfoDouble(_Symbol,SYMBOL_VOLUME_MIN);
   const double step=SymbolInfoDouble(_Symbol,SYMBOL_VOLUME_STEP);
   if(requested+1.0e-12>=remaining)
      return remaining;
   double volume=MathFloor(requested/step+1.0e-12)*step;
   if(volume+1.0e-12<minimum)
      return 0.0;
   if(remaining-volume+1.0e-12<minimum)
      volume=remaining;
   return QFNormalizeVolume(volume);
}

bool QFClosePartial(const double volume)
{
   if(!QFOwnPosition() || volume<=0.0)
      return false;
   const ENUM_POSITION_TYPE type=(ENUM_POSITION_TYPE)PositionGetInteger(POSITION_TYPE);
   const long margin_mode=AccountInfoInteger(ACCOUNT_MARGIN_MODE);
   if(margin_mode==ACCOUNT_MARGIN_MODE_RETAIL_HEDGING)
      return g_trade.PositionClosePartial(_Symbol,volume,InpDeviationPoints);
   return type==POSITION_TYPE_BUY
          ? g_trade.Sell(volume,_Symbol,0.0,0.0,0.0,"QF-partial")
          : g_trade.Buy(volume,_Symbol,0.0,0.0,0.0,"QF-partial");
}

bool QFInCloseBlackout()
{
   MqlDateTime current;
   if(!TimeToStruct(iTime(_Symbol,_Period,0),current))
      return false;
   return current.hour>=@@EOD_HOUR@@;
}

bool QFStopWouldTriggerAtOpen(const bool buy,const double candidate,
                              const double bar_open,const double bar_spread)
{
   if(!QFValid(candidate))
      return true;
   return buy ? (bar_open<=candidate+1.0e-12)
              : (bar_open+bar_spread>=candidate-1.0e-12);
}

// Clamp to stops-level, or EMPTY_VALUE if the stop would already trigger at open
// (matches Rust placeable_stop_candidate / MT5 PositionModify reject).
double QFPlaceableStop(const bool buy,const double raw,const double bar_open,
                       const double bar_spread,const double minimum_distance)
{
   if(!QFValid(raw))
      return EMPTY_VALUE;
   const double candidate=buy ? MathMin(raw,bar_open-minimum_distance)
                              : MathMax(raw,bar_open+bar_spread+minimum_distance);
   if(QFStopWouldTriggerAtOpen(buy,candidate,bar_open,bar_spread))
      return EMPTY_VALUE;
   return candidate;
}

bool QFTightenStop(const bool buy,const double candidate,const double target)
{
   const double current=PositionGetDouble(POSITION_SL);
   if(!QFValid(candidate)
      || (buy && candidate<=current+1.0e-12)
      || (!buy && candidate>=current-1.0e-12))
      return false;
   if(!g_trade.PositionModify(_Symbol,NormalizeDouble(candidate,_Digits),target))
   {
      Print("QuantForge stop modification rejected: ",g_trade.ResultRetcodeDescription());
      return false;
   }
   return true;
}

void QFManagePosition()
{
   if(!QFOwnPosition())
   {
      QFResetPositionState();
      return;
   }
   QFCapturePositionState();
   if(g_initial_volume<=0.0 || g_initial_risk<=0.0)
      return;
   const ENUM_POSITION_TYPE type=(ENUM_POSITION_TYPE)PositionGetInteger(POSITION_TYPE);
   const bool buy=type==POSITION_TYPE_BUY;
   const double entry=PositionGetDouble(POSITION_PRICE_OPEN);
   const double target=PositionGetDouble(POSITION_TP);
   const double sample=QFFavorableSampleSinceEntry(buy);
   QFRatchetPeakFavorable(buy,sample);
   if(!QFValid(g_peak_favorable))
      return;
   const double favorable=g_peak_favorable;
   const double favorable_r=buy ? (favorable-entry)/g_initial_risk
                                : (entry-favorable)/g_initial_risk;
   const double minimum_distance=
      (double)SymbolInfoInteger(_Symbol,SYMBOL_TRADE_STOPS_LEVEL)*_Point;
   // Match Rust: placeable stops only — never clamp a through-market trail onto
   // the open (that invents an immediate runner exit MT5 would reject).
   const double bar_open=iOpen(_Symbol,_Period,0);
   const double bar_spread=(double)iSpread(_Symbol,_Period,0)*_Point;

   if(QFBreakEvenAtR()>0.0 && favorable_r>=QFBreakEvenAtR())
   {
      const double candidate=QFPlaceableStop(buy,entry,bar_open,bar_spread,minimum_distance);
      if(QFValid(candidate))
         QFTightenStop(buy,candidate,target);
   }
   if(QFTrailingKind()>0 && favorable_r>=QFTrailingActivateR())
   {
      const double distance=QFTrailingDistance();
      if(QFValid(distance) && distance>0.0)
      {
         const double raw=buy ? favorable-distance : favorable+distance;
         const double candidate=QFPlaceableStop(buy,raw,bar_open,bar_spread,minimum_distance);
         if(QFValid(candidate))
            QFTightenStop(buy,candidate,target);
      }
   }

   for(int index=0;index<QFPartialCount() && QFOwnPosition();index++)
   {
      if(g_partial_done[index] || favorable_r<QFPartialAtR(index))
         continue;
      const double remaining=PositionGetDouble(POSITION_VOLUME);
      const double volume=QFNormalizePartialVolume(
         g_initial_volume*QFPartialFraction(index),remaining);
      if(volume<=0.0)
         continue;
      if(QFClosePartial(volume))
         g_partial_done[index]=true;
      else
         Print("QuantForge partial exit rejected: ",g_trade.ResultRetcodeDescription());
   }
}

bool QFTimeStopReached()
{
   const int limit=@@TIME_STOP@@;
   if(limit<=0 || !QFOwnPosition())
      return false;
   return g_position_decision_bars>=limit;
}

void QFRecordEquity(const datetime bar_time)
{
   if(g_equity_file==INVALID_HANDLE)
      return;
   // OnDeinit flushes and closes, so per-record flushing only bought crash
   // resilience at the cost of a synchronous disk write per sample.
   FileWrite(g_equity_file,(long)bar_time*1000,
             AccountInfoDouble(ACCOUNT_BALANCE),
             AccountInfoDouble(ACCOUNT_EQUITY));
}

void QFRecordQuoteCloseEquity(const datetime minute_time)
{
   if(g_equity_file==INVALID_HANDLE || !g_quote_bucket_open)
      return;
   const double balance=AccountInfoDouble(ACCOUNT_BALANCE);
   double equity=balance;
   if(QFOwnPosition())
   {
      const ENUM_POSITION_TYPE position_type=
         (ENUM_POSITION_TYPE)PositionGetInteger(POSITION_TYPE);
      const ENUM_ORDER_TYPE order_type=position_type==POSITION_TYPE_BUY
         ? ORDER_TYPE_BUY : ORDER_TYPE_SELL;
      const double mark=position_type==POSITION_TYPE_BUY ? g_bid_close : g_ask_close;
      double floating_profit=0.0;
      if(OrderCalcProfit(order_type,_Symbol,PositionGetDouble(POSITION_VOLUME),
                         PositionGetDouble(POSITION_PRICE_OPEN),mark,floating_profit))
         equity+=floating_profit+PositionGetDouble(POSITION_SWAP);
      else
         equity=AccountInfoDouble(ACCOUNT_EQUITY);
   }
   FileWrite(g_equity_file,(long)minute_time*1000,balance,equity);
}

void QFWriteQuoteBucket()
{
   if(g_quotes_file==INVALID_HANDLE || !g_quote_bucket_open)
      return;
   FileWrite(g_quotes_file,(long)g_quote_minute*1000,
             g_bid_open,g_bid_high,g_bid_low,g_bid_close,
             g_ask_open,g_ask_high,g_ask_low,g_ask_close,
             g_quote_tick_count);
}

void QFCaptureQuoteTick(const MqlTick &tick)
{
   if(g_quotes_file==INVALID_HANDLE || tick.time<=0 || tick.bid<=0.0 || tick.ask<tick.bid)
      return;
   const datetime minute=(datetime)(tick.time-(tick.time%60));
   if(!g_quote_bucket_open || minute!=g_quote_minute)
   {
      QFRecordQuoteCloseEquity(g_quote_minute);
      QFWriteQuoteBucket();
      g_quote_minute=minute;
      g_bid_open=tick.bid; g_bid_high=tick.bid; g_bid_low=tick.bid; g_bid_close=tick.bid;
      g_ask_open=tick.ask; g_ask_high=tick.ask; g_ask_low=tick.ask; g_ask_close=tick.ask;
      g_quote_tick_count=1;
      g_quote_bucket_open=true;
      return;
   }
   g_bid_high=MathMax(g_bid_high,tick.bid);
   g_bid_low=MathMin(g_bid_low,tick.bid);
   g_bid_close=tick.bid;
   g_ask_high=MathMax(g_ask_high,tick.ask);
   g_ask_low=MathMin(g_ask_low,tick.ask);
   g_ask_close=tick.ask;
   g_quote_tick_count++;
}

int OnInit()
{
   if(_Symbol!="@@SYMBOL@@")
   {
      Print("QuantForge export is broker-bound to @@SYMBOL@@, not ",_Symbol);
      return INIT_PARAMETERS_INCORRECT;
   }
   if(!(bool)MQLInfoInteger(MQL_TESTER) && !InpAllowLiveTrading)
   {
      Print("QuantForge live trading is disabled. Explicitly enable InpAllowLiveTrading only after certification.");
      return INIT_FAILED;
   }
   g_trade.SetExpertMagicNumber(InpMagic);
   g_trade.SetDeviationInPoints(InpDeviationPoints);
   g_trade.SetTypeFillingBySymbol(_Symbol);
   ArrayResize(g_partial_done,QFPartialCount());
   QFResetPositionState();
   g_last_bar=iTime(_Symbol,_Period,0);

   if((bool)MQLInfoInteger(MQL_TESTER) && StringLen(InpParityPrefix)>0)
   {
      FolderCreate("QuantForge",FILE_COMMON);
      g_deals_file=FileOpen(InpParityPrefix+"_deals.csv",
                            FILE_WRITE|FILE_CSV|FILE_ANSI|FILE_COMMON,',',CP_UTF8);
      g_equity_file=FileOpen(InpParityPrefix+"_equity.csv",
                             FILE_WRITE|FILE_CSV|FILE_ANSI|FILE_COMMON,',',CP_UTF8);
      g_metadata_file=FileOpen(InpParityPrefix+"_metadata.csv",
                               FILE_WRITE|FILE_CSV|FILE_ANSI|FILE_COMMON,',',CP_UTF8);
      g_quotes_file=FileOpen(InpParityPrefix+"_M1.quotes.csv",
                             FILE_WRITE|FILE_CSV|FILE_ANSI|FILE_COMMON,',',CP_UTF8);
      if(g_deals_file!=INVALID_HANDLE)
         FileWrite(g_deals_file,"deal_ticket","position_id","timestamp_ms","deal_type",
                   "deal_entry","price","volume","profit","commission","swap","fee");
      if(g_equity_file!=INVALID_HANDLE)
         FileWrite(g_equity_file,"timestamp_ms","balance","equity");
      if(g_quotes_file!=INVALID_HANDLE)
         FileWrite(g_quotes_file,"timestamp_ms","bid_open","bid_high","bid_low","bid_close",
                   "ask_open","ask_high","ask_low","ask_close","tick_count");
      if(g_metadata_file!=INVALID_HANDLE)
      {
         FileWrite(g_metadata_file,"property","value");
         FileWrite(g_metadata_file,"strategy_fingerprint","@@STRATEGY_FINGERPRINT@@");
         FileWrite(g_metadata_file,"broker_spec_hash","@@BROKER_FINGERPRINT@@");
         FileWrite(g_metadata_file,"execution_policy_hash","@@EXECUTION_POLICY_FINGERPRINT@@");
         FileWrite(g_metadata_file,"tester_model",@@TESTER_MODEL@@);
         FileWrite(g_metadata_file,"quote_schema_version",1);
         FileWrite(g_metadata_file,"quote_file",InpParityPrefix+"_M1.quotes.csv");
         FileWrite(g_metadata_file,"price_basis","bid_ask");
         FileWrite(g_metadata_file,"execution_model","M1_QUOTES_FROM_TESTER_TICKS");
         FileWrite(g_metadata_file,"terminal_build",TerminalInfoInteger(TERMINAL_BUILD));
         FileWrite(g_metadata_file,"broker",AccountInfoString(ACCOUNT_COMPANY));
         FileWrite(g_metadata_file,"server",AccountInfoString(ACCOUNT_SERVER));
         FileWrite(g_metadata_file,"symbol",_Symbol);
         FileWrite(g_metadata_file,"timeframe",EnumToString(_Period));
         FileWrite(g_metadata_file,"point",@@BROKER_POINT@@);
         FileWrite(g_metadata_file,"tick_size",@@BROKER_TICK_SIZE@@);
         FileWrite(g_metadata_file,"volume_step",@@BROKER_VOLUME_STEP@@);
         FileWrite(g_metadata_file,"magic",InpMagic);
         FileWrite(g_metadata_file,"deviation_points",InpDeviationPoints);
         FileWrite(g_metadata_file,"max_spread_points",InpMaxSpreadPoints);
         FileWrite(g_metadata_file,"estimated_slippage_points_per_side",
                   InpEstimatedSlippagePointsPerSide);
         FileWrite(g_metadata_file,"commission_per_lot_round_turn",
                   InpCommissionPerLotRoundTurn);
         FileWrite(g_metadata_file,"risk_budget",QFRiskBudget());
         FileWrite(g_metadata_file,"initial_deposit",AccountInfoDouble(ACCOUNT_BALANCE));
         FileWrite(g_metadata_file,"account_currency",AccountInfoString(ACCOUNT_CURRENCY));
         // Same token QuantForge uses for bar localization (must match the pack).
         FileWrite(g_metadata_file,"broker_timezone","@@BROKER_TIMEZONE@@");
         FileWrite(g_metadata_file,"server_utc_offset_seconds_at_export",
                   (long)(TimeTradeServer()-TimeGMT()));
         FileWrite(g_metadata_file,"started_server_time",TimeToString(TimeCurrent(),TIME_DATE|TIME_SECONDS));
         FileFlush(g_metadata_file);
      }
   }
   return INIT_SUCCEEDED;
}

void OnDeinit(const int reason)
{
   QFHReleaseHandles();
   QFXReleaseHandles();
   QFRecordQuoteCloseEquity(g_quote_minute);
   QFWriteQuoteBucket();
   if(g_deals_file!=INVALID_HANDLE)
   {
      FileFlush(g_deals_file);
      FileClose(g_deals_file);
   }
   if(g_equity_file!=INVALID_HANDLE)
   {
      FileFlush(g_equity_file);
      FileClose(g_equity_file);
   }
   if(g_metadata_file!=INVALID_HANDLE)
   {
      FileWrite(g_metadata_file,"finished_server_time",TimeToString(TimeCurrent(),TIME_DATE|TIME_SECONDS));
      FileFlush(g_metadata_file);
      FileClose(g_metadata_file);
   }
   if(g_quotes_file!=INVALID_HANDLE)
   {
      FileFlush(g_quotes_file);
      FileClose(g_quotes_file);
   }
}

void OnTradeTransaction(const MqlTradeTransaction &transaction,
                        const MqlTradeRequest &request,
                        const MqlTradeResult &result)
{
   if(transaction.type!=TRADE_TRANSACTION_DEAL_ADD || transaction.deal==0
      || !HistoryDealSelect(transaction.deal))
      return;
   if((ulong)HistoryDealGetInteger(transaction.deal,DEAL_MAGIC)!=InpMagic)
      return;
   const ENUM_DEAL_ENTRY deal_entry=
      (ENUM_DEAL_ENTRY)HistoryDealGetInteger(transaction.deal,DEAL_ENTRY);
   if(deal_entry==DEAL_ENTRY_IN)
   {
      g_initial_volume=0.0;
      QFCapturePositionState();
      // First fill locks the broker day (pending place alone does not).
      QFMarkEntrySignalTaken(iTime(_Symbol,_Period,0));
   }
   if(deal_entry==DEAL_ENTRY_OUT || deal_entry==DEAL_ENTRY_OUT_BY)
   {
      g_last_exit_bar=iTime(_Symbol,_Period,0);
      if(!QFOwnPosition())
         QFResetPositionState();
   }
   if(g_deals_file==INVALID_HANDLE)
      return;
   FileWrite(g_deals_file,
             transaction.deal,
             (ulong)HistoryDealGetInteger(transaction.deal,DEAL_POSITION_ID),
             (long)HistoryDealGetInteger(transaction.deal,DEAL_TIME_MSC),
             EnumToString((ENUM_DEAL_TYPE)HistoryDealGetInteger(transaction.deal,DEAL_TYPE)),
             EnumToString(deal_entry),
             HistoryDealGetDouble(transaction.deal,DEAL_PRICE),
             HistoryDealGetDouble(transaction.deal,DEAL_VOLUME),
             HistoryDealGetDouble(transaction.deal,DEAL_PROFIT),
             HistoryDealGetDouble(transaction.deal,DEAL_COMMISSION),
             HistoryDealGetDouble(transaction.deal,DEAL_SWAP),
             HistoryDealGetDouble(transaction.deal,DEAL_FEE));
   FileFlush(g_deals_file);
}

void OnTick()
{
   MqlTick capture_tick;
   if(SymbolInfoTick(_Symbol,capture_tick))
      QFCaptureQuoteTick(capture_tick);
   const datetime current_bar=iTime(_Symbol,_Period,0);
   if(current_bar<=0 || current_bar==g_last_bar)
      return;
   g_last_bar=current_bar;
   g_decision_bars_seen++;
   if(QFOwnPosition())
      g_position_decision_bars++;

   if(QFFlattenEndOfDay() && QFInCloseBlackout())
   {
      QFCancelOwnOrders();
      if(QFOwnPosition()
         && !g_trade.PositionClose(_Symbol,InpDeviationPoints))
         Print("QuantForge end-of-day exit rejected: ",
               g_trade.ResultRetcodeDescription());
      QFRecordEquity(current_bar);
      return;
   }

   // Hard session: cancel unfilled pending outside [02:00, 19:00).
   if(!QFInMandatoryEntryWindow(current_bar))
      QFCancelOwnOrders();

   QFManagePosition();

   if(QFOwnPosition() && (QFExitSignal(0) || QFTimeStopReached()))
   {
      if(!g_trade.PositionClose(_Symbol,InpDeviationPoints))
         Print("QuantForge exit rejected: ",g_trade.ResultRetcodeDescription());
      QFRecordEquity(current_bar);
      return;
   }
   if(PositionSelect(_Symbol) || QFOwnPendingOrder() || !QFFilters(0))
   {
      QFRecordEquity(current_bar);
      return;
   }
   if(g_last_exit_bar==current_bar)
   {
      QFRecordEquity(current_bar);
      return;
   }
   if(!QFInMandatoryEntryWindow(current_bar))
   {
      QFRecordEquity(current_bar);
      return;
   }
   // MT5 retains pre-test indicator history while QuantForge packs begin at
   // the selected date. Wait for the shared recursive-buffer convergence gate.
   if(g_decision_bars_seen<320)
   {
      QFRecordEquity(current_bar);
      return;
   }
   if(QFEntryDayExhausted(current_bar))
   {
      QFRecordEquity(current_bar);
      return;
   }

   MqlTick tick;
   if(!SymbolInfoTick(_Symbol,tick))
      return;
   const double spread_points=(tick.ask-tick.bid)/_Point;
   if(InpMaxSpreadPoints>0.0 && spread_points>InpMaxSpreadPoints)
   {
      QFRecordEquity(current_bar);
      return;
   }

   const bool long_signal=QFLongSignal(0);
   const bool short_signal=QFShortSignal(0);
   if(long_signal!=short_signal)
   {
      // Place only; day lock happens on fill (DEAL_ENTRY_IN / market position).
      if(QFOpenOrder(long_signal))
      {
         QFCapturePositionState();
         if(QFOwnPosition())
            QFMarkEntrySignalTaken(current_bar);
      }
   }
   QFRecordEquity(current_bar);
}
