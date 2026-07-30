//+------------------------------------------------------------------+
//|                                             SqSupplyDemand.mq5 |
//|                           Copyright © 2026, StrategyQuant s.r.o. |
//+------------------------------------------------------------------+
#property copyright   "Copyright © 2026, StrategyQuant s.r.o."
#property link        "http://www.strategyquant.com"
#property description "Supply and Demand zones (SMC)"
#property indicator_chart_window
#property indicator_buffers 4
#property indicator_plots   4
#property indicator_type1   DRAW_LINE
#property indicator_type2   DRAW_LINE
#property indicator_type3   DRAW_LINE
#property indicator_type4   DRAW_LINE
#property indicator_color1  Tomato
#property indicator_color2  Tomato
#property indicator_color3  MediumSeaGreen
#property indicator_color4  MediumSeaGreen
#property indicator_label1  "SupplyHigh"
#property indicator_label2  "SupplyLow"
#property indicator_label3  "DemandHigh"
#property indicator_label4  "DemandLow"

input int    InpSwingPeriod = 5;
input int    InpATRPeriod   = 14;
input double InpZoneATR     = 0.5;

double SupplyHigh[];
double SupplyLow[];
double DemandHigh[];
double DemandLow[];

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

bool IsSwingHigh(const double &high[], int i, int period)
{
   if(i < period || i + period >= ArraySize(high)) return false;
   double v = high[i];
   for(int k = 1; k <= period; k++)
      if(high[i-k] >= v || high[i+k] >= v) return false;
   return true;
}

bool IsSwingLow(const double &low[], int i, int period)
{
   if(i < period || i + period >= ArraySize(low)) return false;
   double v = low[i];
   for(int k = 1; k <= period; k++)
      if(low[i-k] <= v || low[i+k] <= v) return false;
   return true;
}

int OnInit()
{
   SetIndexBuffer(0, SupplyHigh, INDICATOR_DATA);
   SetIndexBuffer(1, SupplyLow, INDICATOR_DATA);
   SetIndexBuffer(2, DemandHigh, INDICATOR_DATA);
   SetIndexBuffer(3, DemandLow, INDICATOR_DATA);
   IndicatorSetString(INDICATOR_SHORTNAME, "SupplyDemand");
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
   double sH=0, sL=0, dH=0, dL=0;
   if(start > period)
   {
      int p = start - 1;
      sH = SupplyHigh[p]; sL = SupplyLow[p]; dH = DemandHigh[p]; dL = DemandLow[p];
   }

   for(int i = MathMax(start, period); i < rates_total - period && !IsStopped(); i++)
   {
      double atr = CalcATR(high, low, close, i, MathMax(InpATRPeriod, 2));
      double zone = atr * InpZoneATR;

      if(IsSwingHigh(high, i, period))
      {
         sH = high[i];
         sL = high[i] - zone;
      }
      if(IsSwingLow(low, i, period))
      {
         dL = low[i];
         dH = low[i] + zone;
      }

      if(sH > 0 && close[i] > sH) { sH = 0; sL = 0; }
      if(dL > 0 && close[i] < dL) { dH = 0; dL = 0; }

      SupplyHigh[i] = sH;
      SupplyLow[i]  = sL;
      DemandHigh[i] = dH;
      DemandLow[i]  = dL;
   }
   for(int i = MathMax(rates_total - period, start); i < rates_total; i++)
   {
      SupplyHigh[i] = sH; SupplyLow[i] = sL;
      DemandHigh[i] = dH; DemandLow[i] = dL;
   }
   return(rates_total);
}
