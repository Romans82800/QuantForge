//+------------------------------------------------------------------+
//|                                              SqFairValueGap.mq5 |
//|                           Copyright © 2026, StrategyQuant s.r.o. |
//+------------------------------------------------------------------+
#property copyright   "Copyright © 2026, StrategyQuant s.r.o."
#property link        "http://www.strategyquant.com"
#property description "Fair Value Gap zones (SMC)"
#property indicator_chart_window
#property indicator_buffers 6
#property indicator_plots   4
#property indicator_type1   DRAW_LINE
#property indicator_type2   DRAW_LINE
#property indicator_type3   DRAW_LINE
#property indicator_type4   DRAW_LINE
#property indicator_color1  LimeGreen
#property indicator_color2  LimeGreen
#property indicator_color3  OrangeRed
#property indicator_color4  OrangeRed
#property indicator_label1  "BullTop"
#property indicator_label2  "BullBot"
#property indicator_label3  "BearTop"
#property indicator_label4  "BearBot"

input int    InpATRPeriod  = 14;
input double InpMinGapATR  = 0.05;

double BullTop[];
double BullBot[];
double BearTop[];
double BearBot[];
double BullFormed[];
double BearFormed[];

double CalcATR(const double &high[], const double &low[], const double &close[], int i, int period)
{
   if(i < period) return 0;
   double sum = 0;
   for(int k = 0; k < period; k++)
   {
      int idx = i - k;
      double tr;
      if(idx == 0)
         tr = high[idx] - low[idx];
      else
      {
         double hl = high[idx] - low[idx];
         double hc = MathAbs(high[idx] - close[idx + 1]);
         double lc = MathAbs(low[idx] - close[idx + 1]);
         tr = MathMax(hl, MathMax(hc, lc));
      }
      sum += tr;
   }
   return sum / period;
}

int OnInit()
{
   SetIndexBuffer(0, BullTop, INDICATOR_DATA);
   SetIndexBuffer(1, BullBot, INDICATOR_DATA);
   SetIndexBuffer(2, BearTop, INDICATOR_DATA);
   SetIndexBuffer(3, BearBot, INDICATOR_DATA);
   SetIndexBuffer(4, BullFormed, INDICATOR_CALCULATIONS);
   SetIndexBuffer(5, BearFormed, INDICATOR_CALCULATIONS);
   IndicatorSetString(INDICATOR_SHORTNAME, "FVG");
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
   int start = prev_calculated > 0 ? prev_calculated - 1 : 2;
   double actBullTop = 0, actBullBot = 0, actBearTop = 0, actBearBot = 0;
   bool hasBull = false, hasBear = false;

   if(start > 2)
   {
      int p = start - 1;
      if(BullTop[p] > 0 && BullBot[p] > 0) { actBullTop = BullTop[p]; actBullBot = BullBot[p]; hasBull = true; }
      if(BearTop[p] > 0 && BearBot[p] > 0) { actBearTop = BearTop[p]; actBearBot = BearBot[p]; hasBear = true; }
   }

   for(int i = MathMax(start, 2); i < rates_total && !IsStopped(); i++)
   {
      BullFormed[i] = 0;
      BearFormed[i] = 0;
      double atr = CalcATR(high, low, close, i, MathMax(InpATRPeriod, 2));
      double minGap = atr * InpMinGapATR;

      // Bullish FVG: low[i] > high[i-2]
      if(low[i] > high[i-2])
      {
         double gap = low[i] - high[i-2];
         if(gap >= minGap)
         {
            actBullTop = low[i];
            actBullBot = high[i-2];
            hasBull = true;
            BullFormed[i] = 1;
         }
      }
      // Bearish FVG: high[i] < low[i-2]
      if(high[i] < low[i-2])
      {
         double gap = low[i-2] - high[i];
         if(gap >= minGap)
         {
            actBearTop = low[i-2];
            actBearBot = high[i];
            hasBear = true;
            BearFormed[i] = 1;
         }
      }

      // Mitigation
      if(hasBull && low[i] <= actBullBot) hasBull = false;
      if(hasBear && high[i] >= actBearTop) hasBear = false;

      BullTop[i] = hasBull ? actBullTop : 0;
      BullBot[i] = hasBull ? actBullBot : 0;
      BearTop[i] = hasBear ? actBearTop : 0;
      BearBot[i] = hasBear ? actBearBot : 0;
   }
   return(rates_total);
}
