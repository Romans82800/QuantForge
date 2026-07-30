#property copyright "Custom StrategyQuant indicator"
#property link      "https://strategyquant.com"
#property description "Relative Volume Z-Score"
#property indicator_separate_window
#property indicator_buffers 1
#property indicator_plots 1
#property indicator_type1 DRAW_LINE
#property indicator_color1 Purple
#property indicator_label1 "VolumeZ"

input int InpPeriod = 50;
double ExtValue[];

int OnInit()
{
   SetIndexBuffer(0, ExtValue, INDICATOR_DATA);
   IndicatorSetInteger(INDICATOR_DIGITS, 2);
   IndicatorSetString(INDICATOR_SHORTNAME, "VolumeZ(" + string(InpPeriod) + ")");
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
   int period = MathMax(InpPeriod, 5);
   int start = prev_calculated > 0 ? prev_calculated - 1 : 0;

   for(int i = start; i < rates_total && !IsStopped(); i++)
   {
      int bars = MathMin(i + 1, period);
      if(bars < 2) { ExtValue[i] = 0; continue; }

      double sum = 0;
      for(int k = 0; k < bars; k++) sum += (double)tick_volume[i - k];
      double mean = sum / bars;

      double variance = 0;
      for(int k = 0; k < bars; k++)
      {
         double diff = (double)tick_volume[i - k] - mean;
         variance += diff * diff;
      }
      double stdev = MathSqrt(variance / bars);
      ExtValue[i] = stdev > 0 ? ((double)tick_volume[i] - mean) / stdev : 0;
   }

   return(rates_total);
}
