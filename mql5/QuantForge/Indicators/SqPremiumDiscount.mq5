//+------------------------------------------------------------------+
//|                                          SqPremiumDiscount.mq5 |
//|                           Copyright © 2026, StrategyQuant s.r.o. |
//+------------------------------------------------------------------+
#property copyright   "Copyright © 2026, StrategyQuant s.r.o."
#property link        "http://www.strategyquant.com"
#property description "Premium / Discount zones (SMC)"
#property indicator_chart_window
#property indicator_buffers 3
#property indicator_plots   3
#property indicator_type1   DRAW_LINE
#property indicator_type2   DRAW_LINE
#property indicator_type3   DRAW_LINE
#property indicator_color1  Tomato
#property indicator_color2  Silver
#property indicator_color3  MediumSeaGreen
#property indicator_label1  "RangeHigh"
#property indicator_label2  "Equilibrium"
#property indicator_label3  "RangeLow"

input int InpLookback = 50;

double RangeHigh[];
double Equilibrium[];
double RangeLow[];

int OnInit()
{
   SetIndexBuffer(0, RangeHigh, INDICATOR_DATA);
   SetIndexBuffer(1, Equilibrium, INDICATOR_DATA);
   SetIndexBuffer(2, RangeLow, INDICATOR_DATA);
   PlotIndexSetInteger(0, PLOT_DRAW_BEGIN, InpLookback);
   PlotIndexSetInteger(1, PLOT_DRAW_BEGIN, InpLookback);
   PlotIndexSetInteger(2, PLOT_DRAW_BEGIN, InpLookback);
   IndicatorSetString(INDICATOR_SHORTNAME, "PremDisc");
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
   int lookback = MathMax(InpLookback, 5);
   int start = prev_calculated > 0 ? prev_calculated - 1 : 0;

   for(int i = MathMax(start, lookback - 1); i < rates_total && !IsStopped(); i++)
   {
      double hi = high[i];
      double lo = low[i];
      for(int k = 0; k < lookback; k++)
      {
         if(high[i-k] > hi) hi = high[i-k];
         if(low[i-k] < lo) lo = low[i-k];
      }
      RangeHigh[i] = hi;
      RangeLow[i] = lo;
      Equilibrium[i] = (hi + lo) / 2.0;
   }
   return(rates_total);
}
