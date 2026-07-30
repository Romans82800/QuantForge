//+------------------------------------------------------------------+
//|                                         SqChangeOfCharacter.mq5 |
//|                           Copyright © 2026, StrategyQuant s.r.o. |
//+------------------------------------------------------------------+
#property copyright   "Copyright © 2026, StrategyQuant s.r.o."
#property link        "http://www.strategyquant.com"
#property description "Change of Character (SMC)"
#property indicator_separate_window
#property indicator_buffers 4
#property indicator_plots   2
#property indicator_type1   DRAW_HISTOGRAM
#property indicator_type2   DRAW_LINE
#property indicator_color1  Gold
#property indicator_color2  Silver
#property indicator_label1  "CHoCH"
#property indicator_label2  "Structure"

input int InpSwingPeriod = 5;

double CHoCHBuf[];
double StructBuf[];
double SwingHighBuf[];
double SwingLowBuf[];

int structureTrend = 0;

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
   SetIndexBuffer(0, CHoCHBuf, INDICATOR_DATA);
   SetIndexBuffer(1, StructBuf, INDICATOR_DATA);
   SetIndexBuffer(2, SwingHighBuf, INDICATOR_CALCULATIONS);
   SetIndexBuffer(3, SwingLowBuf, INDICATOR_CALCULATIONS);
   IndicatorSetString(INDICATOR_SHORTNAME, "CHoCH");
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
      structureTrend = (int)StructBuf[start - 1];
   }

   for(int i = MathMax(start, period); i < rates_total && !IsStopped(); i++)
   {
      CHoCHBuf[i] = 0;
      int check = i - period;
      if(check >= period && IsSwingHigh(high, check, period)) lastSH = high[check];
      if(check >= period && IsSwingLow(low, check, period)) lastSL = low[check];
      SwingHighBuf[i] = lastSH;
      SwingLowBuf[i] = lastSL;

      if(lastSH > 0 && close[i] > lastSH && close[i-1] <= lastSH)
      {
         if(structureTrend <= 0) CHoCHBuf[i] = 1;
         structureTrend = 1;
      }
      if(lastSL > 0 && close[i] < lastSL && close[i-1] >= lastSL)
      {
         if(structureTrend >= 0) CHoCHBuf[i] = -1;
         structureTrend = -1;
      }
      StructBuf[i] = structureTrend;
   }
   return(rates_total);
}
