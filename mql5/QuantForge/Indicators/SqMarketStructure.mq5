#property copyright "Custom StrategyQuant indicator"
#property link      "https://strategyquant.com"
#property description "Market Structure Swing/BOS"
#property indicator_chart_window
#property indicator_buffers 1
#property indicator_plots 1
#property indicator_type1 DRAW_LINE
#property indicator_color1 Crimson
#property indicator_label1 "MarketStructure"

input int InpStrength = 2;
input int InpLookback = 100;
input int InpMode = 0;
double ExtValue[];

bool IsSwingHigh(const int shift, const int current, const int strength, const double &high[])
{
   double v = high[shift];
   for(int i = shift - strength; i <= shift + strength; i++)
   {
      if(i < 0 || i > current || i == shift) continue;
      if(high[i] >= v) return false;
   }
   return true;
}

bool IsSwingLow(const int shift, const int current, const int strength, const double &low[])
{
   double v = low[shift];
   for(int i = shift - strength; i <= shift + strength; i++)
   {
      if(i < 0 || i > current || i == shift) continue;
      if(low[i] <= v) return false;
   }
   return true;
}

int OnInit()
{
   SetIndexBuffer(0, ExtValue, INDICATOR_DATA);
   IndicatorSetInteger(INDICATOR_DIGITS, _Digits);
   IndicatorSetString(INDICATOR_SHORTNAME, "MarketStructure");
   return(INIT_SUCCEEDED);
}

int OnCalculate(const int rates_total,
                const int prev_calculated,
                const datetime &time[],
                const double &open[],
                const double &high[],
                const double &low[],
                const double &close[],
                const long &tick_volume[],
                const long &volume[],
                const int &spread[])
{
   int strength = MathMax(InpStrength, 1);
   int lookback = MathMax(InpLookback, 20);
   int start = prev_calculated > 0 ? prev_calculated - 1 : 0;

   for(int bar = start; bar < rates_total && !IsStopped(); bar++)
   {
      double lastHigh = 0, prevHigh = 0, lastLow = 0, prevLow = 0;
      int foundHighs = 0, foundLows = 0;
      int bars = MathMin(bar, lookback);

      for(int k = strength; k <= bars; k++)
      {
         int idx = bar - k;
         if(foundHighs < 2 && IsSwingHigh(idx, bar, strength, high))
         {
            if(foundHighs == 0) lastHigh = high[idx];
            else prevHigh = high[idx];
            foundHighs++;
         }
         if(foundLows < 2 && IsSwingLow(idx, bar, strength, low))
         {
            if(foundLows == 0) lastLow = low[idx];
            else prevLow = low[idx];
            foundLows++;
         }
         if(foundHighs >= 2 && foundLows >= 2) break;
      }

      if(InpMode == 0) ExtValue[bar] = lastHigh;
      else if(InpMode == 1) ExtValue[bar] = lastLow;
      else if(InpMode == 2) ExtValue[bar] = lastHigh > 0 && close[bar] > lastHigh ? 1 : (lastLow > 0 && close[bar] < lastLow ? -1 : 0);
      else
      {
         int trend = 0;
         if(foundHighs >= 2 && foundLows >= 2)
         {
            if(lastHigh > prevHigh && lastLow > prevLow) trend = 1;
            else if(lastHigh < prevHigh && lastLow < prevLow) trend = -1;
         }
         ExtValue[bar] = trend;
      }
   }

   return(rates_total);
}
