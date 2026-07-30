//+------------------------------------------------------------------+
//|                                                        SqVWAP.mq5|
//|                            Copyright © @2021 StrategyQuant s.r.o.|
//|                                     http://www.strategyquant.com |
//+------------------------------------------------------------------+
#property  copyright "Copyright © @2021 StrategyQuant s.r.o."
#property  link      "http://www.strategyquant.com"

#property indicator_chart_window
#property indicator_buffers 1
#property indicator_plots 1

#property indicator_label1  "SqVWAP"
#property indicator_type1  DRAW_LINE
#property indicator_color1 Red

//---- indicator parameters
input int VWAPPeriod=10;

//---- buffers
double IndiBuffer[];

//---- handle

void OnInit()
  {
     
   ArraySetAsSeries(IndiBuffer, true);
   SetIndexBuffer(0, IndiBuffer,INDICATOR_DATA);
   PlotIndexSetInteger(0,PLOT_DRAW_BEGIN,VWAPPeriod);
   
  
//--- indicator short name
   string short_name="SqVWAP("+VWAPPeriod+")";
   IndicatorSetString(INDICATOR_SHORTNAME,short_name);
//---- end of initialization function
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
   ArraySetAsSeries(open, true);
   ArraySetAsSeries(high, true);
   ArraySetAsSeries(low, true);
   ArraySetAsSeries(close, true);
   ArraySetAsSeries(tick_volume, true);
   
   if(rates_total < VWAPPeriod) return(0);
   
   int limit;
    
   
   if(prev_calculated > 0) limit = rates_total - prev_calculated + 1;
   else {
      for(int a=0; a<rates_total; a++){
         IndiBuffer[a] = 0.0;

      }
      
      limit = rates_total - VWAPPeriod;
   }
 //--- main indicator loop
 
   for(int i=limit-1; i>=0; i--) {
   
      double ohlcAvg = 0;
      
      double __ohlcvTotal = 0;
      double __volumeTotal = 0;
      
      
      for(int p = 0; p< VWAPPeriod; p++){
              
         ohlcAvg = (open[i+p]+high[i+p]+low[i+p]+close[i+p])/4;
             
         __ohlcvTotal = __ohlcvTotal + (ohlcAvg*(double) tick_volume[i+p]);
			__volumeTotal =__volumeTotal + (double) tick_volume[i+p];
      }
      double vwap =0;
      if(__volumeTotal!=0){
      
      vwap = __ohlcvTotal/__volumeTotal;
  
      }
      
      IndiBuffer[i] =vwap;
       
   }
   return(rates_total);
  }
//+------------------------------------------------------------------+



double getIndicatorValue(int indyHandle, int bufferIndex, int shift){
   double buffer[];
   
   if(CopyBuffer(indyHandle, bufferIndex, shift, 1, buffer) < 0) { 
      //--- if the copying fails, tell the error code 
      PrintFormat("Failed to copy data from the indicator, error code %d", GetLastError()); 
      //--- quit with zero result - it means that the indicator is considered as not calculated 
      return(0); 
   } 
   
   double val = buffer[0];
   return val;
}