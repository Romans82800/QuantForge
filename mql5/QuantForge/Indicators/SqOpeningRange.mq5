//+------------------------------------------------------------------+
//|                                             SqOpeningRange.mq5 |
//|                           Copyright © 2026, StrategyQuant s.r.o. |
//+------------------------------------------------------------------+
#property copyright   "Copyright © 2026, StrategyQuant s.r.o."
#property link        "http://www.strategyquant.com"
#property description "Opening Range (ORB)"
#property indicator_chart_window
#property indicator_buffers 4
#property indicator_plots   3
#property indicator_type1   DRAW_LINE
#property indicator_type2   DRAW_LINE
#property indicator_type3   DRAW_LINE
#property indicator_color1  DodgerBlue
#property indicator_color2  Silver
#property indicator_color3  OrangeRed
#property indicator_label1  "ORHigh"
#property indicator_label2  "ORMid"
#property indicator_label3  "ORLow"

input int InpStartHour   = 9;
input int InpStartMinute = 30;
input int InpRangeMinutes = 30;

double ORHigh[];
double ORMid[];
double ORLow[];
double ORComplete[];

int OnInit()
{
   SetIndexBuffer(0, ORHigh, INDICATOR_DATA);
   SetIndexBuffer(1, ORMid, INDICATOR_DATA);
   SetIndexBuffer(2, ORLow, INDICATOR_DATA);
   SetIndexBuffer(3, ORComplete, INDICATOR_CALCULATIONS);
   IndicatorSetString(INDICATOR_SHORTNAME, "OpeningRange");
   return(INIT_SUCCEEDED);
}

int BarMinutes(const datetime &time[], int i)
{
   MqlDateTime dt;
   TimeToStruct(time[i], dt);
   return dt.hour * 60 + dt.min;
}

int DayKey(const datetime &time[], int i)
{
   MqlDateTime dt;
   TimeToStruct(time[i], dt);
   return dt.year * 10000 + dt.mon * 100 + dt.day;
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
   int startMin = InpStartHour * 60 + InpStartMinute;
   int endMin = startMin + MathMax(InpRangeMinutes, 1);
   int start = prev_calculated > 0 ? prev_calculated - 1 : 0;

   int curDay = -1;
   double orH = 0, orL = 0;
   bool forming = false;
   bool complete = false;

   if(start > 0)
   {
      orH = ORHigh[start-1];
      orL = ORLow[start-1];
      complete = ORComplete[start-1] > 0;
      curDay = DayKey(time, start-1);
      forming = (orH > 0 && orL > 0 && !complete);
   }

   for(int i = start; i < rates_total && !IsStopped(); i++)
   {
      int day = DayKey(time, i);
      int mins = BarMinutes(time, i);

      if(day != curDay)
      {
         curDay = day;
         orH = 0;
         orL = 0;
         forming = false;
         complete = false;
      }

      if(!complete && mins >= startMin && mins < endMin)
      {
         forming = true;
         if(orH <= 0)
         {
            orH = high[i];
            orL = low[i];
         }
         else
         {
            if(high[i] > orH) orH = high[i];
            if(low[i] < orL)  orL = low[i];
         }
      }
      else if(!complete && forming && mins >= endMin)
      {
         complete = true;
      }

      ORHigh[i] = (orH > 0 && orL > 0) ? orH : 0;
      ORLow[i]  = (orH > 0 && orL > 0) ? orL : 0;
      ORMid[i]  = (orH > 0 && orL > 0) ? (orH + orL) / 2.0 : 0;
      ORComplete[i] = complete ? 1 : 0;
   }
   return(rates_total);
}
