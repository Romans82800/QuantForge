//+------------------------------------------------------------------+
//|                                            SqChoppinessIndex.mq5 |
//|                           Copyright © 2026, StrategyQuant s.r.o. |
//|                                     http://www.strategyquant.com |
//+------------------------------------------------------------------+
#property copyright   "Copyright © 2026, StrategyQuant s.r.o."
#property link        "http://www.strategyquant.com"
#property description "Choppiness Index"
#property indicator_separate_window
#property indicator_buffers 1
#property indicator_plots   1
#property indicator_type1   DRAW_LINE
#property indicator_color1  Purple
#property indicator_label1  "Choppiness"

input int InpPeriod = 14;

double ExtChopBuffer[];

int OnInit()
{
   int period = MathMax(InpPeriod, 2);
   SetIndexBuffer(0, ExtChopBuffer, INDICATOR_DATA);
   IndicatorSetInteger(INDICATOR_DIGITS, 2);
   IndicatorSetString(INDICATOR_SHORTNAME, "CHOP(" + string(period) + ")");
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
   int period = MathMax(InpPeriod, 2);
   int start = prev_calculated > 0 ? prev_calculated - 1 : 0;
   double logPeriod = MathLog10(period);

   for(int i = start; i < rates_total && !IsStopped(); i++)
   {
      int bars = MathMin(i + 1, period);
      if(bars < period || i < 1)
      {
         ExtChopBuffer[i] = 0;
         continue;
      }

      double atrSum = 0;
      double highest = high[i];
      double lowest = low[i];

      for(int k = 0; k < period; k++)
      {
         int idx = i - k;
         double h = high[idx];
         double l = low[idx];
         double pc = close[idx - 1];
         double tr = MathMax(h - l, MathMax(MathAbs(h - pc), MathAbs(l - pc)));
         atrSum += tr;
         if(h > highest) highest = h;
         if(l < lowest) lowest = l;
      }

      double range = highest - lowest;
      if(range <= 0 || atrSum <= 0 || logPeriod <= 0)
         ExtChopBuffer[i] = 0;
      else
      {
         double chop = 100.0 * MathLog10(atrSum / range) / logPeriod;
         if(chop < 0) chop = 0;
         if(chop > 100) chop = 100;
         ExtChopBuffer[i] = chop;
      }
   }

   return(rates_total);
}
