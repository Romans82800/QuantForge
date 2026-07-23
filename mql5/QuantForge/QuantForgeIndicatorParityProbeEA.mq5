#property strict
#property version   "1.00"
#property description "Non-trading MT5 reference-buffer exporter for QuantForge indicator parity."

input int InpBars=5000;
input int InpPeriod=14;
input string InpOutputFile="QuantForge\\indicator_parity.csv";
input int InpMaximumAttempts=120;

int g_sma=INVALID_HANDLE;
int g_ema=INVALID_HANDLE;
int g_wma=INVALID_HANDLE;
int g_rsi=INVALID_HANDLE;
int g_atr=INVALID_HANDLE;
int g_stddev=INVALID_HANDLE;
int g_attempts=0;
bool g_finished=false;

void ReleaseHandles()
{
   int handles[]={g_sma,g_ema,g_wma,g_rsi,g_atr,g_stddev};
   for(int index=0;index<ArraySize(handles);index++)
      if(handles[index]!=INVALID_HANDLE)
         IndicatorRelease(handles[index]);
   g_sma=g_ema=g_wma=g_rsi=g_atr=g_stddev=INVALID_HANDLE;
}

bool BuffersReady()
{
   int handles[]={g_sma,g_ema,g_wma,g_rsi,g_atr,g_stddev};
   for(int index=0;index<ArraySize(handles);index++)
      if(handles[index]==INVALID_HANDLE || BarsCalculated(handles[index])<InpBars)
         return false;
   return true;
}

bool CopyReferenceBuffer(const int handle,const int count,double &values[])
{
   ArrayResize(values,count);
   ResetLastError();
   const int copied=CopyBuffer(handle,0,1,count,values);
   if(copied!=count)
   {
      Print("QuantForge indicator probe CopyBuffer failed. copied=",copied,
            " expected=",count," error=",GetLastError());
      return false;
   }
   return true;
}

string IsoServerTime(const datetime value)
{
   string text=TimeToString(value,TIME_DATE|TIME_SECONDS);
   StringReplace(text,".","-");
   StringReplace(text," ","T");
   return text;
}

bool ExportReferencePack()
{
   MqlRates rates[];
   ArrayResize(rates,InpBars);
   ResetLastError();
   const int count=CopyRates(_Symbol,_Period,1,InpBars,rates);
   if(count<InpPeriod+2)
   {
      Print("QuantForge indicator probe CopyRates is not ready. copied=",count,
            " error=",GetLastError());
      return false;
   }
   ArrayResize(rates,count);

   double sma[],ema[],wma[],rsi[],atr_values[],stddev[];
   if(!CopyReferenceBuffer(g_sma,count,sma)
      || !CopyReferenceBuffer(g_ema,count,ema)
      || !CopyReferenceBuffer(g_wma,count,wma)
      || !CopyReferenceBuffer(g_rsi,count,rsi)
      || !CopyReferenceBuffer(g_atr,count,atr_values)
      || !CopyReferenceBuffer(g_stddev,count,stddev))
      return false;

   const string partial=InpOutputFile+".partial";
   FileDelete(partial,FILE_COMMON);
   const int file=FileOpen(partial,
                           FILE_WRITE|FILE_CSV|FILE_ANSI|FILE_COMMON,
                           ',',CP_UTF8);
   if(file==INVALID_HANDLE)
   {
      Print("QuantForge indicator probe FileOpen failed. error=",GetLastError());
      return false;
   }

   FileWrite(file,
             "timestamp_ms","server_time","open","high","low","close",
             "sma","ema","wma","rsi","atr","donchian_high","donchian_low",
             "highest_close","lowest_close","standard_deviation","zscore",
             "percentile_in_range","rate_of_change","session_hour","day_of_week",
             "terminal_build","broker","server","symbol","timeframe","period");

   const int digits=(int)SymbolInfoInteger(_Symbol,SYMBOL_DIGITS);
   for(int index=0;index<count;index++)
   {
      // CopyRates and CopyBuffer place the oldest requested value at index zero.
      const int shift=count-index;
      const int high_shift=iHighest(_Symbol,_Period,MODE_HIGH,InpPeriod,shift);
      const int low_shift=iLowest(_Symbol,_Period,MODE_LOW,InpPeriod,shift);
      const int highest_close_shift=iHighest(_Symbol,_Period,MODE_CLOSE,InpPeriod,shift);
      const int lowest_close_shift=iLowest(_Symbol,_Period,MODE_CLOSE,InpPeriod,shift);
      if(high_shift<0 || low_shift<0 || highest_close_shift<0 || lowest_close_shift<0)
      {
         FileClose(file);
         FileDelete(partial,FILE_COMMON);
         Print("QuantForge indicator probe extreme lookup failed at shift ",shift);
         return false;
      }

      const double donchian_high=iHigh(_Symbol,_Period,high_shift);
      const double donchian_low=iLow(_Symbol,_Period,low_shift);
      const double highest_close=iClose(_Symbol,_Period,highest_close_shift);
      const double lowest_close=iClose(_Symbol,_Period,lowest_close_shift);
      const double zscore=stddev[index]>0.0
                          ? (rates[index].close-sma[index])/stddev[index]
                          : EMPTY_VALUE;
      const double percentile=highest_close>lowest_close
                              ? (rates[index].close-lowest_close)
                                /(highest_close-lowest_close)*100.0
                              : EMPTY_VALUE;
      const double previous_close=iClose(_Symbol,_Period,shift+InpPeriod);
      const double roc=previous_close!=0.0
                       ? (rates[index].close/previous_close-1.0)*100.0
                       : EMPTY_VALUE;
      MqlDateTime time_parts;
      TimeToStruct(rates[index].time,time_parts);

      FileWrite(file,
                (long)rates[index].time*1000,
                IsoServerTime(rates[index].time),
                DoubleToString(rates[index].open,digits),
                DoubleToString(rates[index].high,digits),
                DoubleToString(rates[index].low,digits),
                DoubleToString(rates[index].close,digits),
                DoubleToString(sma[index],16),
                DoubleToString(ema[index],16),
                DoubleToString(wma[index],16),
                DoubleToString(rsi[index],16),
                DoubleToString(atr_values[index],16),
                DoubleToString(donchian_high,16),
                DoubleToString(donchian_low,16),
                DoubleToString(highest_close,16),
                DoubleToString(lowest_close,16),
                DoubleToString(stddev[index],16),
                DoubleToString(zscore,16),
                DoubleToString(percentile,16),
                DoubleToString(roc,16),
                time_parts.hour,
                time_parts.day_of_week,
                (int)TerminalInfoInteger(TERMINAL_BUILD),
                AccountInfoString(ACCOUNT_COMPANY),
                AccountInfoString(ACCOUNT_SERVER),
                _Symbol,
                EnumToString(_Period),
                InpPeriod);
   }

   FileFlush(file);
   FileClose(file);
   ResetLastError();
   if(!FileMove(partial,FILE_COMMON,InpOutputFile,FILE_COMMON|FILE_REWRITE))
   {
      Print("QuantForge indicator probe could not publish output. error=",GetLastError());
      FileDelete(partial,FILE_COMMON);
      return false;
   }
   Print("QuantForge exported ",count," indicator reference rows to Common\\Files\\",
         InpOutputFile);
   return true;
}

void AttemptExport()
{
   if(g_finished)
      return;
   g_attempts++;
   if(BuffersReady() && ExportReferencePack())
   {
      g_finished=true;
      EventKillTimer();
      ExpertRemove();
      return;
   }
   if(g_attempts>=InpMaximumAttempts)
   {
      g_finished=true;
      EventKillTimer();
      Print("QuantForge indicator probe timed out waiting for reference buffers.");
      ExpertRemove();
   }
}

int OnInit()
{
   if(!(bool)MQLInfoInteger(MQL_TESTER))
   {
      Print("QuantForgeIndicatorParityProbeEA must run in the MT5 strategy tester.");
      return INIT_FAILED;
   }
   if(InpBars<100 || InpPeriod<2 || InpMaximumAttempts<1)
   {
      Print("QuantForge indicator probe inputs are invalid.");
      return INIT_PARAMETERS_INCORRECT;
   }

   g_sma=iMA(_Symbol,_Period,InpPeriod,0,MODE_SMA,PRICE_CLOSE);
   g_ema=iMA(_Symbol,_Period,InpPeriod,0,MODE_EMA,PRICE_CLOSE);
   g_wma=iMA(_Symbol,_Period,InpPeriod,0,MODE_LWMA,PRICE_CLOSE);
   g_rsi=iRSI(_Symbol,_Period,InpPeriod,PRICE_CLOSE);
   g_atr=iATR(_Symbol,_Period,InpPeriod);
   g_stddev=iStdDev(_Symbol,_Period,InpPeriod,0,MODE_SMA,PRICE_CLOSE);
   if(g_sma==INVALID_HANDLE || g_ema==INVALID_HANDLE || g_wma==INVALID_HANDLE
      || g_rsi==INVALID_HANDLE || g_atr==INVALID_HANDLE || g_stddev==INVALID_HANDLE)
   {
      Print("QuantForge indicator probe could not create indicator handles. error=",GetLastError());
      ReleaseHandles();
      return INIT_FAILED;
   }
   EventSetTimer(1);
   return INIT_SUCCEEDED;
}

void OnTick()
{
   AttemptExport();
}

void OnTimer()
{
   AttemptExport();
}

void OnDeinit(const int reason)
{
   EventKillTimer();
   ReleaseHandles();
}
