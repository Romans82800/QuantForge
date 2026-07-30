//+------------------------------------------------------------------+
//|                                          SqBreakOfStructure.mq5 |
//|                           Copyright © 2026, StrategyQuant s.r.o. |
//+------------------------------------------------------------------+
#property copyright   "Copyright © 2026, StrategyQuant s.r.o."
#property link        "http://www.strategyquant.com"
#property description "Break of Structure (SMC)"
#property indicator_separate_window
#property indicator_buffers 5
#property indicator_plots   3
#property indicator_type1   DRAW_HISTOGRAM
#property indicator_type2   DRAW_LINE
#property indicator_type3   DRAW_LINE
#property indicator_color1  DodgerBlue
#property indicator_color2  LimeGreen
#property indicator_color3  OrangeRed
#property indicator_label1  "BOS"
#property indicator_label2  "SwingHigh"
#property indicator_label3  "SwingLow"

input int InpSwingPeriod = 5;

double BOSBuffer[];
double SwingHighBuf[];
double SwingLowBuf[];
double BullBOS[];
double BearBOS[];

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
   SetIndexBuffer(0, BOSBuffer, INDICATOR_DATA);
   SetIndexBuffer(1, SwingHighBuf, INDICATOR_DATA);
   SetIndexBuffer(2, SwingLowBuf, INDICATOR_DATA);
   SetIndexBuffer(3, BullBOS, INDICATOR_CALCULATIONS);
   SetIndexBuffer(4, BearBOS, INDICATOR_CALCULATIONS);
   IndicatorSetString(INDICATOR_SHORTNAME, "BOS");
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
   if(start > period)
   {
      lastSH = SwingHighBuf[start - 1];
      lastSL = SwingLowBuf[start - 1];
   }

   for(int i = MathMax(start, period); i < rates_total && !IsStopped(); i++)
   {
      BullBOS[i] = 0;
      BearBOS[i] = 0;
      BOSBuffer[i] = 0;

      int checkIdx = i - period;
      if(checkIdx >= period && IsSwingHigh(high, checkIdx, period))
         lastSH = high[checkIdx];
      if(checkIdx >= period && IsSwingLow(low, checkIdx, period))
         lastSL = low[checkIdx];

      SwingHighBuf[i] = lastSH;
      SwingLowBuf[i]  = lastSL;

      if(lastSH > 0 && close[i] > lastSH && close[i-1] <= lastSH)
      {
         BOSBuffer[i] = 1;
         BullBOS[i] = 1;
      }
      if(lastSL > 0 && close[i] < lastSL && close[i-1] >= lastSL)
      {
         BOSBuffer[i] = -1;
         BearBOS[i] = 1;
      }
   }
   return(rates_total);
}
