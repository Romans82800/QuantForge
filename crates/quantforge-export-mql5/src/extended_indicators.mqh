//+------------------------------------------------------------------+
//| QuantForge extended indicators                                    |
//|                                                                   |
//| Shared by both export styles and injected by the generator.        |
//| Each function mirrors the matching arm of                          |
//| quantforge_eval::features::calculate_indicator_series_with_clock.  |
//| Fractional parameters arrive as tenths so the IR stays integral.   |
//+------------------------------------------------------------------+

double QFXBuffer(const int handle,const int buffer,const int shift)
{
   if(handle==INVALID_HANDLE || shift<0)
      return EMPTY_VALUE;
   double values[];
   const int copied=CopyBuffer(handle,buffer,shift,1,values);
   if(copied<1)
      return EMPTY_VALUE;
   return values[0];
}

// Handles live for the whole run. Recreating one per condition atom makes MT5
// rebuild the indicator's history on every tick, which is the single largest
// cost in a generated expert. This cache is deliberately independent of the SQX
// runtime's own cache because both files are inlined into one source.
#define QFX_MAX_HANDLES 32

string g_qfx_handle_keys[QFX_MAX_HANDLES];
int    g_qfx_handle_values[QFX_MAX_HANDLES];
int    g_qfx_handle_count=0;

int QFXCachedHandle(const string key)
{
   for(int index=0;index<g_qfx_handle_count;index++)
      if(g_qfx_handle_keys[index]==key)
         return g_qfx_handle_values[index];
   return -2;
}

int QFXRemember(const string key,const int handle)
{
   if(g_qfx_handle_count<QFX_MAX_HANDLES)
   {
      g_qfx_handle_keys[g_qfx_handle_count]=key;
      g_qfx_handle_values[g_qfx_handle_count]=handle;
      g_qfx_handle_count++;
   }
   return handle;
}

void QFXReleaseHandles()
{
   for(int index=0;index<g_qfx_handle_count;index++)
      if(g_qfx_handle_values[index]!=INVALID_HANDLE)
         IndicatorRelease(g_qfx_handle_values[index]);
   g_qfx_handle_count=0;
}

int QFXMacdHandle(const ENUM_APPLIED_PRICE source,const int fast_period,
                  const int slow_period,const int signal_period)
{
   const string key="MACD|"+IntegerToString(fast_period)+"|"+IntegerToString(slow_period)
                    +"|"+IntegerToString(signal_period)+"|"+IntegerToString((int)source);
   const int cached=QFXCachedHandle(key);
   if(cached!=-2)
      return cached;
   return QFXRemember(key,iMACD(_Symbol,_Period,fast_period,slow_period,signal_period,source));
}

int QFXBandsHandle(const ENUM_APPLIED_PRICE source,const int period,const double deviation)
{
   const string key="BANDS|"+IntegerToString(period)+"|"+DoubleToString(deviation,4)
                    +"|"+IntegerToString((int)source);
   const int cached=QFXCachedHandle(key);
   if(cached!=-2)
      return cached;
   return QFXRemember(key,iBands(_Symbol,_Period,period,0,deviation,source));
}

int QFXIchimokuHandle(const int tenkan,const int kijun,const int senkou)
{
   const string key="ICHIMOKU|"+IntegerToString(tenkan)+"|"+IntegerToString(kijun)
                    +"|"+IntegerToString(senkou);
   const int cached=QFXCachedHandle(key);
   if(cached!=-2)
      return cached;
   return QFXRemember(key,iIchimoku(_Symbol,_Period,tenkan,kijun,senkou));
}

int QFXCciHandle(const int period)
{
   const string key="CCI|"+IntegerToString(period);
   const int cached=QFXCachedHandle(key);
   if(cached!=-2)
      return cached;
   return QFXRemember(key,iCCI(_Symbol,_Period,period,PRICE_TYPICAL));
}

int QFXRsiHandle(const int period)
{
   const string key="QFXRSI|"+IntegerToString(period);
   const int cached=QFXCachedHandle(key);
   if(cached!=-2)
      return cached;
   return QFXRemember(key,iRSI(_Symbol,_Period,period,PRICE_CLOSE));
}

//--- MACD -----------------------------------------------------------
// iMACD MAIN is EMA(fast) - EMA(slow) and SIGNAL is its EMA, which is the
// same construction the Rust evaluator uses.
double QFMacdMain(const ENUM_APPLIED_PRICE source,const int fast_period,
                  const int slow_period,const int signal_period,const int shift)
{
   return QFXBuffer(QFXMacdHandle(source,fast_period,slow_period,signal_period),
                    MAIN_LINE,shift);
}

double QFMacdSignal(const ENUM_APPLIED_PRICE source,const int fast_period,
                    const int slow_period,const int signal_period,const int shift)
{
   return QFXBuffer(QFXMacdHandle(source,fast_period,slow_period,signal_period),
                    SIGNAL_LINE,shift);
}

double QFMacdHistogram(const ENUM_APPLIED_PRICE source,const int fast_period,
                       const int slow_period,const int signal_period,const int shift)
{
   const int handle=QFXMacdHandle(source,fast_period,slow_period,signal_period);
   if(handle==INVALID_HANDLE || shift<0)
      return EMPTY_VALUE;
   double main_values[],signal_values[];
   const int main_copied=CopyBuffer(handle,MAIN_LINE,shift,1,main_values);
   const int signal_copied=CopyBuffer(handle,SIGNAL_LINE,shift,1,signal_values);
   if(main_copied<1 || signal_copied<1)
      return EMPTY_VALUE;
   if(!QFValid(main_values[0]) || !QFValid(signal_values[0]))
      return EMPTY_VALUE;
   return main_values[0]-signal_values[0];
}

//--- Bollinger ------------------------------------------------------
double QFBollinger(const ENUM_APPLIED_PRICE source,const int period,
                   const int deviation_tenths,const int buffer,const int shift)
{
   const double deviation=(double)deviation_tenths/10.0;
   return QFXBuffer(QFXBandsHandle(source,period,deviation),buffer,shift);
}

double QFBollingerMid(const ENUM_APPLIED_PRICE source,const int period,const int shift)
{
   return QFBollinger(source,period,20,BASE_LINE,shift);
}

double QFBollingerUpper(const ENUM_APPLIED_PRICE source,const int period,
                        const int deviation_tenths,const int shift)
{
   return QFBollinger(source,period,deviation_tenths,UPPER_BAND,shift);
}

double QFBollingerLower(const ENUM_APPLIED_PRICE source,const int period,
                        const int deviation_tenths,const int shift)
{
   return QFBollinger(source,period,deviation_tenths,LOWER_BAND,shift);
}

double QFBollingerBandwidth(const ENUM_APPLIED_PRICE source,const int period,
                            const int deviation_tenths,const int shift)
{
   const double deviation=(double)deviation_tenths/10.0;
   const int handle=QFXBandsHandle(source,period,deviation);
   if(handle==INVALID_HANDLE || shift<0)
      return EMPTY_VALUE;
   double base[],upper[],lower[];
   const int base_copied=CopyBuffer(handle,BASE_LINE,shift,1,base);
   const int upper_copied=CopyBuffer(handle,UPPER_BAND,shift,1,upper);
   const int lower_copied=CopyBuffer(handle,LOWER_BAND,shift,1,lower);
   if(base_copied<1 || upper_copied<1 || lower_copied<1)
      return EMPTY_VALUE;
   if(!QFValid(base[0]) || !QFValid(upper[0]) || !QFValid(lower[0]) || base[0]==0.0)
      return EMPTY_VALUE;
   return (upper[0]-lower[0])/base[0]*100.0;
}

//--- Ichimoku -------------------------------------------------------
// Tenkan and Kijun are read in place. The Senkou spans are plotted
// kijun bars ahead of the data that formed them, so the value visible on a
// bar comes from buffer index shift+kijun.
double QFIchimokuTenkan(const int period,const int shift)
{
   return QFXBuffer(QFXIchimokuHandle(period,26,52),TENKANSEN_LINE,shift);
}

double QFIchimokuKijun(const int period,const int shift)
{
   return QFXBuffer(QFXIchimokuHandle(9,period,52),KIJUNSEN_LINE,shift);
}

double QFIchimokuSenkouA(const int tenkan_period,const int kijun_period,const int shift)
{
   return QFXBuffer(QFXIchimokuHandle(tenkan_period,kijun_period,52),
                    SENKOUSPANA_LINE,shift+kijun_period);
}

double QFIchimokuSenkouB(const int period,const int kijun_period,const int shift)
{
   return QFXBuffer(QFXIchimokuHandle(9,kijun_period,period),
                    SENKOUSPANB_LINE,shift+kijun_period);
}

//--- CCI ------------------------------------------------------------
double QFCci(const int period,const int shift)
{
   return QFXBuffer(QFXCciHandle(period),0,shift);
}

//--- VWAP -----------------------------------------------------------
// Rolling volume-weighted typical price. Feeds without tick volume fall back
// to equal weights so the buffer stays defined.
double QFVwap(const int period,const int shift)
{
   if(period<1 || shift<0 || Bars(_Symbol,_Period)<shift+period)
      return EMPTY_VALUE;
   double weighted=0.0;
   double total=0.0;
   for(int index=shift;index<shift+period;index++)
     {
      const double typical=(iHigh(_Symbol,_Period,index)
                            +iLow(_Symbol,_Period,index)
                            +iClose(_Symbol,_Period,index))/3.0;
      double weight=(double)iVolume(_Symbol,_Period,index);
      if(weight<=0.0)
         weight=1.0;
      weighted+=typical*weight;
      total+=weight;
     }
   if(total<=0.0)
      return EMPTY_VALUE;
   return weighted/total;
}

//--- QQE ------------------------------------------------------------
// QQE needs recursive averages over the whole RSI history, so the series is
// built once per bar from a single bulk CopyBuffer and cached. Reading it the
// way the other helpers do (one handle per value) would be orders of magnitude
// slower inside the tester.
#define QF_QQE_MAX_BARS 4096

double   _qf_qqe_line[];
double   _qf_qqe_trail[];
int      _qf_qqe_count=0;
int      _qf_qqe_rsi_period=-1;
int      _qf_qqe_smoothing=-1;
int      _qf_qqe_factor_tenths=-1;
datetime _qf_qqe_stamp=0;

void QFXEmaSparse(const double &values[],const int count,const int period,double &output[])
{
   ArrayResize(output,count);
   for(int index=0;index<count;index++)
      output[index]=EMPTY_VALUE;
   if(period<1 || count<=0)
      return;
   int first=-1;
   for(int index=0;index<count;index++)
      if(QFValid(values[index]))
        {
         first=index;
         break;
        }
   if(first<0 || first+period>count)
      return;
   double sum=0.0;
   for(int index=first;index<first+period;index++)
      sum+=values[index];
   double previous=sum/(double)period;
   output[first+period-1]=previous;
   const double alpha=2.0/((double)period+1.0);
   for(int index=first+period;index<count;index++)
     {
      if(!QFValid(values[index]))
         continue;
      previous=alpha*values[index]+(1.0-alpha)*previous;
      output[index]=previous;
     }
}

// factor_tenths of -1 means the caller only needs the smoothed line, which does
// not depend on the factor; any cached factor is then acceptable.
void QFXQqeBuild(const int rsi_period,const int smoothing_period,const int factor_tenths)
{
   const datetime stamp=iTime(_Symbol,_Period,0);
   const bool same_shape=_qf_qqe_rsi_period==rsi_period
                         && _qf_qqe_smoothing==smoothing_period
                         && _qf_qqe_stamp==stamp
                         && _qf_qqe_count>0;
   if(same_shape && (factor_tenths<0 || _qf_qqe_factor_tenths==factor_tenths))
      return;

   const int effective_factor=factor_tenths<0
                              ? (_qf_qqe_factor_tenths>0 ? _qf_qqe_factor_tenths : 42)
                              : factor_tenths;

   _qf_qqe_rsi_period=rsi_period;
   _qf_qqe_smoothing=smoothing_period;
   _qf_qqe_factor_tenths=effective_factor;
   _qf_qqe_stamp=stamp;
   _qf_qqe_count=0;

   const int available=Bars(_Symbol,_Period);
   int count=available<QF_QQE_MAX_BARS ? available : QF_QQE_MAX_BARS;
   if(count<=rsi_period+smoothing_period+2)
      return;

   const int handle=QFXRsiHandle(rsi_period);
   if(handle==INVALID_HANDLE)
      return;
   double rsi[];
   // Oldest-first ordering so the recursion runs the same direction as Rust.
   const int copied=CopyBuffer(handle,0,0,count,rsi);
   if(copied<1)
      return;
   count=copied;

   double smoothed[];
   QFXEmaSparse(rsi,count,smoothing_period,smoothed);

   double absolute[];
   ArrayResize(absolute,count);
   for(int index=0;index<count;index++)
      absolute[index]=EMPTY_VALUE;
   for(int index=1;index<count;index++)
      if(QFValid(smoothed[index]) && QFValid(smoothed[index-1]))
         absolute[index]=MathAbs(smoothed[index-1]-smoothed[index]);

   int wilder=rsi_period*2-1;
   if(wilder<1)
      wilder=1;
   double smoothed_absolute[];
   QFXEmaSparse(absolute,count,wilder,smoothed_absolute);
   double deviation[];
   QFXEmaSparse(smoothed_absolute,count,wilder,deviation);

   ArrayResize(_qf_qqe_line,count);
   ArrayResize(_qf_qqe_trail,count);
   for(int index=0;index<count;index++)
     {
      _qf_qqe_line[index]=smoothed[index];
      _qf_qqe_trail[index]=EMPTY_VALUE;
     }

   const double factor=(double)effective_factor/10.0;
   double long_band=EMPTY_VALUE;
   double short_band=EMPTY_VALUE;
   bool trend_up=true;
   for(int index=1;index<count;index++)
     {
      if(!QFValid(smoothed[index]) || !QFValid(smoothed[index-1]) || !QFValid(deviation[index]))
         continue;
      const double current=smoothed[index];
      const double previous=smoothed[index-1];
      const double room=deviation[index]*factor;
      const double new_long=current-room;
      const double new_short=current+room;

      if(QFValid(long_band) && previous>long_band && current>long_band)
         long_band=MathMax(long_band,new_long);
      else
         long_band=new_long;

      if(QFValid(short_band) && previous<short_band && current<short_band)
         short_band=MathMin(short_band,new_short);
      else
         short_band=new_short;

      if(current>short_band)
         trend_up=true;
      else
         if(current<long_band)
            trend_up=false;

      _qf_qqe_trail[index]=trend_up ? long_band : short_band;
     }
   _qf_qqe_count=count;
}

double QFQqeLine(const int rsi_period,const int smoothing_period,const int shift)
{
   QFXQqeBuild(rsi_period,smoothing_period,-1);
   if(_qf_qqe_count<=0 || shift<0 || shift>=_qf_qqe_count)
      return EMPTY_VALUE;
   // CopyBuffer returned oldest-first; convert the series shift to that index.
   return _qf_qqe_line[_qf_qqe_count-1-shift];
}

double QFQqeTrail(const int rsi_period,const int smoothing_period,
                  const int factor_tenths,const int shift)
{
   QFXQqeBuild(rsi_period,smoothing_period,factor_tenths);
   if(_qf_qqe_count<=0 || shift<0 || shift>=_qf_qqe_count)
      return EMPTY_VALUE;
   return _qf_qqe_trail[_qf_qqe_count-1-shift];
}
