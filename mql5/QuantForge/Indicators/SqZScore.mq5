//+------------------------------------------------------------------+
//|                                                     SqZScore.mq5 |
//|                           Copyright © 2026, StrategyQuant s.r.o. |
//|                                     http://www.strategyquant.com |
//+------------------------------------------------------------------+
#property copyright   "Copyright © 2026, StrategyQuant s.r.o."
#property link        "http://www.strategyquant.com"
#property description "Z-Score of Close"
#property indicator_separate_window
#property indicator_buffers 1
#property indicator_plots   1
#property indicator_type1   DRAW_LINE
#property indicator_color1  Orange
#property indicator_label1  "ZScore"

input int InpPeriod = 20;

double ExtZScoreBuffer[];

int OnInit()
{
   int period = MathMax(InpPeriod, 2);
   SetIndexBuffer(0, ExtZScoreBuffer, INDICATOR_DATA);
   IndicatorSetInteger(INDICATOR_DIGITS, 2);
   IndicatorSetString(INDICATOR_SHORTNAME, "ZScore(" + string(period) + ")");
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

   for(int i = start; i < rates_total && !IsStopped(); i++)
   {
      int bars = MathMin(i + 1, period);
      if(bars < period)
      {
         ExtZScoreBuffer[i] = 0;
         continue;
      }

      double sum = 0;
      for(int k = 0; k < period; k++)
         sum += close[i - k];
      double mean = sum / period;

      double sqSum = 0;
      for(int k = 0; k < period; k++)
      {
         double diff = close[i - k] - mean;
         sqSum += diff * diff;
      }

      double stdDev = MathSqrt(sqSum / period);
      ExtZScoreBuffer[i] = (stdDev > 0) ? (close[i] - mean) / stdDev : 0;
   }

   return(rates_total);
}
