#property copyright "Custom StrategyQuant indicator"
#property link      "https://strategyquant.com"
#property description "ATR Percentile"
#property indicator_separate_window
#property indicator_buffers 1
#property indicator_plots 1
#property indicator_type1 DRAW_LINE
#property indicator_color1 DodgerBlue
#property indicator_label1 "ATRPercentile"

input int InpATRPeriod = 14;
input int InpLookback = 100;
double ExtValue[];

double AtrAt(const int i, const double &high[], const double &low[], const double &close[])
{
   int period = MathMax(InpATRPeriod, 2);
   int bars = MathMin(i + 1, period);
   if(bars <= 0) return 0;

   double sum = 0;
   for(int k = 0; k < bars; k++)
   {
      int idx = i - k;
      double tr = high[idx] - low[idx];
      if(idx > 0)
         tr = MathMax(MathAbs(high[idx] - close[idx - 1]), MathMax(tr, MathAbs(low[idx] - close[idx - 1])));
      sum += tr;
   }
   return sum / bars;
}

int OnInit()
{
   SetIndexBuffer(0, ExtValue, INDICATOR_DATA);
   IndicatorSetInteger(INDICATOR_DIGITS, 2);
   IndicatorSetString(INDICATOR_SHORTNAME, "ATRPercentile(" + string(InpATRPeriod) + "," + string(InpLookback) + ")");
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
   int lookback = MathMax(InpLookback, 10);
   int start = prev_calculated > 0 ? prev_calculated - 1 : 0;

   for(int i = start; i < rates_total && !IsStopped(); i++)
   {
      double currentAtr = AtrAt(i, high, low, close);
      int bars = MathMin(i + 1, lookback);
      if(bars < 2 || currentAtr <= 0) { ExtValue[i] = 0; continue; }

      int belowOrEqual = 0;
      for(int k = 0; k < bars; k++)
      {
         if(AtrAt(i - k, high, low, close) <= currentAtr) belowOrEqual++;
      }
      ExtValue[i] = 100.0 * belowOrEqual / bars;
   }

   return(rates_total);
}
