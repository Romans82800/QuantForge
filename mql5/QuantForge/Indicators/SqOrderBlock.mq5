//+------------------------------------------------------------------+
//|                                               SqOrderBlock.mq5 |
//|                           Copyright © 2026, StrategyQuant s.r.o. |
//+------------------------------------------------------------------+
#property copyright   "Copyright © 2026, StrategyQuant s.r.o."
#property link        "http://www.strategyquant.com"
#property description "Order Block zones (SMC)"
#property indicator_chart_window
#property indicator_buffers 4
#property indicator_plots   4
#property indicator_type1   DRAW_LINE
#property indicator_type2   DRAW_LINE
#property indicator_type3   DRAW_LINE
#property indicator_type4   DRAW_LINE
#property indicator_color1  DodgerBlue
#property indicator_color2  DodgerBlue
#property indicator_color3  Crimson
#property indicator_color4  Crimson
#property indicator_label1  "BullOBHigh"
#property indicator_label2  "BullOBLow"
#property indicator_label3  "BearOBHigh"
#property indicator_label4  "BearOBLow"

input int    InpATRPeriod       = 14;
input double InpDisplacementATR = 1.5;
input int    InpLookback        = 20;

double BullOBHigh[];
double BullOBLow[];
double BearOBHigh[];
double BearOBLow[];

double CalcATR(const double &high[], const double &low[], const double &close[], int i, int period)
{
   if(i < period) return 0;
   double sum = 0;
   for(int k = 0; k < period; k++)
   {
      int idx = i - k;
      double tr;
      if(idx == 0) tr = high[idx] - low[idx];
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
   SetIndexBuffer(0, BullOBHigh, INDICATOR_DATA);
   SetIndexBuffer(1, BullOBLow, INDICATOR_DATA);
   SetIndexBuffer(2, BearOBHigh, INDICATOR_DATA);
   SetIndexBuffer(3, BearOBLow, INDICATOR_DATA);
   IndicatorSetString(INDICATOR_SHORTNAME, "OrderBlock");
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
   int start = prev_calculated > 0 ? prev_calculated - 1 : 0;
   double bH=0, bL=0, sH=0, sL=0;
   if(start > 0)
   {
      int p = start - 1;
      bH = BullOBHigh[p]; bL = BullOBLow[p]; sH = BearOBHigh[p]; sL = BearOBLow[p];
   }

   for(int i = start; i < rates_total && !IsStopped(); i++)
   {
      double atr = CalcATR(high, low, close, i, MathMax(InpATRPeriod, 2));
      double threshold = atr * InpDisplacementATR;
      double body = MathAbs(close[i] - open[i]);

      if(body >= threshold)
      {
         if(close[i] > open[i])
         {
            for(int j = 1; j <= InpLookback && i - j >= 0; j++)
            {
               int idx = i - j;
               if(close[idx] < open[idx])
               {
                  bH = high[idx]; bL = low[idx];
                  break;
               }
            }
         }
         else if(close[i] < open[i])
         {
            for(int j = 1; j <= InpLookback && i - j >= 0; j++)
            {
               int idx = i - j;
               if(close[idx] > open[idx])
               {
                  sH = high[idx]; sL = low[idx];
                  break;
               }
            }
         }
      }

      if(bH > 0 && close[i] < bL) { bH = 0; bL = 0; }
      if(sH > 0 && close[i] > sH) { sH = 0; sL = 0; }

      BullOBHigh[i] = bH;
      BullOBLow[i]  = bL;
      BearOBHigh[i] = sH;
      BearOBLow[i]  = sL;
   }
   return(rates_total);
}
