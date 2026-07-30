//+------------------------------------------------------------------+
//|                                                       SqRVOL.mq5 |
//|                           Copyright © 2026, StrategyQuant s.r.o. |
//|                                     http://www.strategyquant.com |
//+------------------------------------------------------------------+
#property copyright   "Copyright © 2026, StrategyQuant s.r.o."
#property link        "http://www.strategyquant.com"
#property description "Relative Volume"
#property indicator_separate_window
#property indicator_buffers 1
#property indicator_plots   1
#property indicator_type1   DRAW_LINE
#property indicator_color1  DodgerBlue
#property indicator_label1  "RVOL"

input int InpLookback = 20;

double ExtRVOLBuffer[];

int OnInit()
{
   int lookback = MathMax(InpLookback, 2);
   SetIndexBuffer(0, ExtRVOLBuffer, INDICATOR_DATA);
   IndicatorSetInteger(INDICATOR_DIGITS, 2);
   IndicatorSetString(INDICATOR_SHORTNAME, "RVOL(" + string(lookback) + ")");
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
   int lookback = MathMax(InpLookback, 2);
   int start = prev_calculated > 0 ? prev_calculated - 1 : 0;

   for(int i = start; i < rates_total && !IsStopped(); i++)
   {
      int bars = MathMin(i + 1, lookback);
      if(bars < lookback)
      {
         ExtRVOLBuffer[i] = 0;
         continue;
      }

      double barVol = (double)(volume[i] > 0 ? volume[i] : tick_volume[i]);
      double sumVol = 0;
      for(int k = 0; k < lookback; k++)
         sumVol += (double)(volume[i - k] > 0 ? volume[i - k] : tick_volume[i - k]);

      double avgVol = sumVol / lookback;
      ExtRVOLBuffer[i] = (avgVol > 0) ? barVol / avgVol : 0;
   }

   return(rates_total);
}
