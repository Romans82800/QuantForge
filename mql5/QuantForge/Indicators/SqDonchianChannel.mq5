//+------------------------------------------------------------------+
//|                                            SqDonchianChannel.mq5 |
//|                           Copyright © 2026, StrategyQuant s.r.o. |
//|                                     http://www.strategyquant.com |
//+------------------------------------------------------------------+
#property copyright   "Copyright © 2026, StrategyQuant s.r.o."
#property link        "http://www.strategyquant.com"
#property description "Donchian Channel"
#property indicator_chart_window
#property indicator_buffers 3
#property indicator_plots   3
#property indicator_type1   DRAW_LINE
#property indicator_type2   DRAW_LINE
#property indicator_type3   DRAW_LINE
#property indicator_color1  LimeGreen
#property indicator_color2  Red
#property indicator_color3  Silver
#property indicator_label1  "Upper"
#property indicator_label2  "Lower"
#property indicator_label3  "Middle"

input int InpPeriod = 20;

double ExtUpperBuffer[];
double ExtLowerBuffer[];
double ExtMiddleBuffer[];

int OnInit()
{
   int period = MathMax(InpPeriod, 2);
   SetIndexBuffer(0, ExtUpperBuffer, INDICATOR_DATA);
   SetIndexBuffer(1, ExtLowerBuffer, INDICATOR_DATA);
   SetIndexBuffer(2, ExtMiddleBuffer, INDICATOR_DATA);
   PlotIndexSetInteger(0, PLOT_DRAW_BEGIN, period - 1);
   PlotIndexSetInteger(1, PLOT_DRAW_BEGIN, period - 1);
   PlotIndexSetInteger(2, PLOT_DRAW_BEGIN, period - 1);
   IndicatorSetString(INDICATOR_SHORTNAME, "Donchian(" + string(period) + ")");
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
         ExtUpperBuffer[i] = 0;
         ExtLowerBuffer[i] = 0;
         ExtMiddleBuffer[i] = 0;
         continue;
      }

      double highest = high[i];
      double lowest = low[i];
      for(int k = 0; k < period; k++)
      {
         if(high[i - k] > highest) highest = high[i - k];
         if(low[i - k] < lowest) lowest = low[i - k];
      }

      ExtUpperBuffer[i] = highest;
      ExtLowerBuffer[i] = lowest;
      ExtMiddleBuffer[i] = (highest + lowest) / 2.0;
   }

   return(rates_total);
}
