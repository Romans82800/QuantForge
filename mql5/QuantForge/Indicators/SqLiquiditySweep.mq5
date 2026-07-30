//+------------------------------------------------------------------+
//|                                            SqLiquiditySweep.mq5 |
//|                           Copyright © 2026, StrategyQuant s.r.o. |
//+------------------------------------------------------------------+
#property copyright   "Copyright © 2026, StrategyQuant s.r.o."
#property link        "http://www.strategyquant.com"
#property description "Liquidity Sweeps (SMC)"
#property indicator_separate_window
#property indicator_buffers 4
#property indicator_plots   4
#property indicator_type1   DRAW_HISTOGRAM
#property indicator_type2   DRAW_HISTOGRAM
#property indicator_type3   DRAW_NONE
#property indicator_type4   DRAW_NONE
#property indicator_color1  LimeGreen
#property indicator_color2  OrangeRed
#property indicator_label1  "BullSweep"
#property indicator_label2  "BearSweep"

input int InpSwingPeriod = 5;

double BullSweep[];
double BearSweep[];
double SwingHighBuf[];
double SwingLowBuf[];


bool IsSwingHigh(const double &high[], int i, int period)
{
   if(i < period) return false;
   double v = high[i];
   for(int k = 1; k <= period; k++)
   {
      if(i - k < 0) return false;
      if(high[i-k] >= v) return false;
   }
   for(int k = 1; k <= period; k++)
   {
      if(i + k >= ArraySize(high)) return false;
      if(high[i+k] >= v) return false;
   }
   return true;
}

bool IsSwingLow(const double &low[], int i, int period)
{
   if(i < period) return false;
   double v = low[i];
   for(int k = 1; k <= period; k++)
   {
      if(i - k < 0) return false;
      if(low[i-k] <= v) return false;
   }
   for(int k = 1; k <= period; k++)
   {
      if(i + k >= ArraySize(low)) return false;
      if(low[i+k] <= v) return false;
   }
   return true;
}


int OnInit()
{
   SetIndexBuffer(0, BullSweep, INDICATOR_DATA);
   SetIndexBuffer(1, BearSweep, INDICATOR_DATA);
   SetIndexBuffer(2, SwingHighBuf, INDICATOR_DATA);
   SetIndexBuffer(3, SwingLowBuf, INDICATOR_DATA);
   PlotIndexSetInteger(2, PLOT_DRAW_TYPE, DRAW_NONE);
   PlotIndexSetInteger(3, PLOT_DRAW_TYPE, DRAW_NONE);
   IndicatorSetString(INDICATOR_SHORTNAME, "LiqSweep");
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
   int period = MathMax(InpSwingPeriod, 2);
   int start = prev_calculated > 0 ? prev_calculated - 1 : period;
   double lastSH = 0, lastSL = 0;
   if(start > 0) { lastSH = SwingHighBuf[start-1]; lastSL = SwingLowBuf[start-1]; }

   for(int i = MathMax(start, period); i < rates_total && !IsStopped(); i++)
   {
      BullSweep[i] = 0;
      BearSweep[i] = 0;
      int check = i - period;
      if(check >= period && IsSwingHigh(high, check, period)) lastSH = high[check];
      if(check >= period && IsSwingLow(low, check, period)) lastSL = low[check];
      SwingHighBuf[i] = lastSH;
      SwingLowBuf[i] = lastSL;

      if(lastSL > 0 && low[i] < lastSL && close[i] > lastSL) BullSweep[i] = 1;
      if(lastSH > 0 && high[i] > lastSH && close[i] < lastSH) BearSweep[i] = 1;
   }
   return(rates_total);
}
