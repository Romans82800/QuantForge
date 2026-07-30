//+------------------------------------------------------------------+
//|                                               SqParkinsonVol.mq5 |
//|                           Copyright © 2026, StrategyQuant s.r.o. |
//|                                     http://www.strategyquant.com |
//+------------------------------------------------------------------+
#property copyright   "Copyright © 2026, StrategyQuant s.r.o."
#property link        "http://www.strategyquant.com"
#property description "Parkinson Volatility"
#property version     "1.00"
#property indicator_separate_window
#property indicator_buffers 1
#property indicator_plots   1
#property indicator_type1   DRAW_LINE
#property indicator_color1  Teal
#property indicator_label1  "ParkinsonVol"

input int InpPeriod = 14;

double ExtParkinsonBuffer[];

int OnInit()
{
   int period = MathMax(InpPeriod, 2);
   SetIndexBuffer(0, ExtParkinsonBuffer, INDICATOR_DATA);
   ArraySetAsSeries(ExtParkinsonBuffer, true);
   IndicatorSetInteger(INDICATOR_DIGITS, _Digits);
   IndicatorSetString(INDICATOR_SHORTNAME, "PVOL(" + IntegerToString(period) + ")");
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
   if(rates_total < period)
      return(0);

   ArraySetAsSeries(high, true);
   ArraySetAsSeries(low, true);
   ArraySetAsSeries(close, true);

   int limit = (prev_calculated > 0) ? rates_total - prev_calculated : rates_total - period;
   if(limit < 0)
      limit = 0;

   const double parkinsonConst = 1.0 / (4.0 * MathLog(2.0));

   for(int i = limit; i >= 0; i--)
   {
      double sum = 0;
      int count = 0;
      for(int k = 0; k < period; k++)
      {
         double h = high[i + k];
         double l = low[i + k];
         if(l <= 0)
            continue;
         double logHl = MathLog(h / l);
         sum += parkinsonConst * logHl * logHl;
         count++;
      }

      if(count <= 0)
         ExtParkinsonBuffer[i] = 0;
      else
         ExtParkinsonBuffer[i] = MathSqrt(sum / count) * close[i];
   }

   return(rates_total);
}
