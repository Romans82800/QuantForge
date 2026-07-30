// QuantForge SQX-style runtime helpers.
// Bar-open timing and indicator access aligned with StrategyQuant MT5 exports.

#ifndef __QUANTFORGE_SQX_RUNTIME_MQH__
#define __QUANTFORGE_SQX_RUNTIME_MQH__

#include <Trade/Trade.mqh>

datetime g_sq_last_bar_time=0;
bool _sqIsBarOpen=false;

void checkBarOpen()
{
   const datetime current_bar_time=iTime(_Symbol,_Period,1);
   _sqIsBarOpen=false;
   if(current_bar_time<=0)
      return;
   if(g_sq_last_bar_time==0)
   {
      g_sq_last_bar_time=current_bar_time;
      _sqIsBarOpen=true;
      return;
   }
   if(current_bar_time!=g_sq_last_bar_time)
   {
      g_sq_last_bar_time=current_bar_time;
      _sqIsBarOpen=true;
   }
}

bool sqValid(const double value)
{
   return value!=EMPTY_VALUE && MathIsValidNumber(value);
}

bool sqGreater(const double left,const double right)
{
   return sqValid(left) && sqValid(right) && left>right;
}

bool sqLess(const double left,const double right)
{
   return sqValid(left) && sqValid(right) && left<right;
}

bool sqCrossAbove(const double current_left,const double current_right,
                  const double previous_left,const double previous_right)
{
   return sqValid(current_left) && sqValid(current_right)
          && sqValid(previous_left) && sqValid(previous_right)
          && current_left>current_right && previous_left<=previous_right;
}

bool sqCrossBelow(const double current_left,const double current_right,
                  const double previous_left,const double previous_right)
{
   return sqValid(current_left) && sqValid(current_right)
          && sqValid(previous_left) && sqValid(previous_right)
          && current_left<current_right && previous_left>=previous_right;
}

double sqBufferValue(const int handle,const int shift)
{
   if(handle==INVALID_HANDLE || shift<0)
      return EMPTY_VALUE;
   double values[1];
   if(CopyBuffer(handle,0,shift,1,values)!=1)
      return EMPTY_VALUE;
   return values[0];
}

// Indicator handles are cached for the life of the run.  Creating and releasing
// a handle per condition atom forces MT5 to rebuild the indicator's history on
// every tick, which dwarfs the cost of the strategy logic itself and is the main
// reason a generated expert crawls in the Strategy Tester.
#define SQ_MAX_INDICATOR_HANDLES 64

string g_sq_handle_keys[SQ_MAX_INDICATOR_HANDLES];
int    g_sq_handle_values[SQ_MAX_INDICATOR_HANDLES];
int    g_sq_handle_count=0;

int sqHandleSlot(const string key)
{
   for(int index=0;index<g_sq_handle_count;index++)
      if(g_sq_handle_keys[index]==key)
         return index;
   return -1;
}

int sqRemember(const string key,const int handle)
{
   if(g_sq_handle_count<SQ_MAX_INDICATOR_HANDLES)
   {
      g_sq_handle_keys[g_sq_handle_count]=key;
      g_sq_handle_values[g_sq_handle_count]=handle;
      g_sq_handle_count++;
   }
   return handle;
}

void sqReleaseHandles()
{
   for(int index=0;index<g_sq_handle_count;index++)
      if(g_sq_handle_values[index]!=INVALID_HANDLE)
         IndicatorRelease(g_sq_handle_values[index]);
   g_sq_handle_count=0;
}

int sqCustomHandle(const string name,const int first)
{
   const string key=name+"|"+IntegerToString(first);
   const int slot=sqHandleSlot(key);
   if(slot>=0)
      return g_sq_handle_values[slot];
   return sqRemember(key,iCustom(_Symbol,_Period,name,first));
}

int sqCustomHandle2(const string name,const int first,const int second)
{
   const string key=name+"|"+IntegerToString(first)+"|"+IntegerToString(second);
   const int slot=sqHandleSlot(key);
   if(slot>=0)
      return g_sq_handle_values[slot];
   return sqRemember(key,iCustom(_Symbol,_Period,name,first,second));
}

int sqMaHandle(const ENUM_MA_METHOD method,const ENUM_APPLIED_PRICE source,const int period)
{
   const string key="MA|"+IntegerToString(period)+"|"+IntegerToString((int)method)
                    +"|"+IntegerToString((int)source);
   const int slot=sqHandleSlot(key);
   if(slot>=0)
      return g_sq_handle_values[slot];
   return sqRemember(key,iMA(_Symbol,_Period,period,0,method,source));
}

int sqRsiHandle(const ENUM_APPLIED_PRICE source,const int period)
{
   const string key="RSI|"+IntegerToString(period)+"|"+IntegerToString((int)source);
   const int slot=sqHandleSlot(key);
   if(slot>=0)
      return g_sq_handle_values[slot];
   return sqRemember(key,iRSI(_Symbol,_Period,period,source));
}

int sqStdDevHandle(const ENUM_APPLIED_PRICE source,const int period)
{
   const string key="STDDEV|"+IntegerToString(period)+"|"+IntegerToString((int)source);
   const int slot=sqHandleSlot(key);
   if(slot>=0)
      return g_sq_handle_values[slot];
   return sqRemember(key,iStdDev(_Symbol,_Period,period,0,MODE_SMA,source));
}

double sqPrice(const int field,const int shift)
{
   if(shift<0 || Bars(_Symbol,_Period)<=shift)
      return EMPTY_VALUE;
   if(field==0) return iOpen(_Symbol,_Period,shift);
   if(field==1) return iHigh(_Symbol,_Period,shift);
   if(field==2) return iLow(_Symbol,_Period,shift);
   return iClose(_Symbol,_Period,shift);
}

double sqMa(const ENUM_MA_METHOD method,const ENUM_APPLIED_PRICE source,
            const int period,const int shift)
{
   return sqBufferValue(sqMaHandle(method,source,period),shift);
}

double sqRsi(const ENUM_APPLIED_PRICE source,const int period,const int shift)
{
   return sqBufferValue(sqRsiHandle(source,period),shift);
}

double sqSqAtr(const int period,const int shift)
{
   return sqBufferValue(sqCustomHandle("SqATR",period),shift);
}

double sqSqAdx(const int period,const int shift)
{
   return sqBufferValue(sqCustomHandle("SqADX",period),shift);
}

double sqDirectionalIndex(const int period,const int shift,const int buffer)
{
   const int handle=sqCustomHandle("SqADX",period);
   double values[1];
   if(handle==INVALID_HANDLE || shift<0 || CopyBuffer(handle,buffer,shift,1,values)!=1)
      return EMPTY_VALUE;
   return values[0];
}

double sqPlusDi(const int period,const int shift)
{
   return sqDirectionalIndex(period,shift,1);
}

double sqMinusDi(const int period,const int shift)
{
   return sqDirectionalIndex(period,shift,2);
}

double sqSqRoc(const int period,const int shift)
{
   return sqBufferValue(sqCustomHandle("SqROC",period),shift);
}

double sqExtreme(const ENUM_SERIESMODE mode,const int field,const int period,const int shift,
                 const bool maximum)
{
   // `field` is the QuantForge QFPrice code (0=open,1=high,2=low,3=close).
   // SqHighest/SqLowest expect ENUM_APPLIED_PRICE (PRICE_CLOSE=1, PRICE_HIGH=3, …).
   int price=PRICE_CLOSE;
   if(field==0) price=PRICE_OPEN;
   else if(field==1) price=PRICE_HIGH;
   else if(field==2) price=PRICE_LOW;
   const string indicator=maximum ? "SqHighest" : "SqLowest";
   return sqBufferValue(sqCustomHandle2(indicator,period,price),shift);
}

double sqStdDev(const ENUM_APPLIED_PRICE source,const int period,const int shift)
{
   return sqBufferValue(sqStdDevHandle(source,period),shift);
}

double sqSwingBaseZoneExtreme(const int swing_left,const int swing_right,const int base_bars,
                              const int shift,const bool zone_high)
{
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

double sqSwingBaseZoneHigh(const int swing_left,const int swing_right,const int base_bars,
                           const int shift)
{
   return sqSwingBaseZoneExtreme(swing_left,swing_right,base_bars,shift,true);
}

double sqSwingBaseZoneLow(const int swing_left,const int swing_right,const int base_bars,
                          const int shift)
{
   return sqSwingBaseZoneExtreme(swing_left,swing_right,base_bars,shift,false);
}

double sqSqZScore(const int period,const int shift)
{
   return sqBufferValue(sqCustomHandle("SqZScore",period),shift);
}

double sqSqAtrPercentile(const int atr_period,const int lookback,const int shift)
{
   return sqBufferValue(sqCustomHandle2("SqATRPercentile",atr_period,lookback),shift);
}

double sqLiquiditySweepScore(const int period,const int shift)
{
   const int handle=sqCustomHandle("SqLiquiditySweep",period);
   if(handle==INVALID_HANDLE || shift<0)
      return 0.0;
   double bull[1];
   double bear[1];
   if(CopyBuffer(handle,0,shift,1,bull)!=1 || CopyBuffer(handle,1,shift,1,bear)!=1)
      return 0.0;
   if(bull[0]>0.0)
      return 1.0;
   if(bear[0]>0.0)
      return -1.0;
   return 0.0;
}

#endif
