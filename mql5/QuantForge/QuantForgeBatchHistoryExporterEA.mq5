#property strict
#property version   "1.00"
#property description "Non-trading, chunked IC Markets OHLCV and broker-metadata batch exporter for QuantForge."

input string InpSymbols="AUDUSD,GBPUSD,GBPJPY,NZDUSD,EURJPY,EURNZD,EURGBP,USDJPY,USDCHF,US500,XAUUSD";
input string InpTimeframes="M1,H1";
input datetime InpFrom=D'2020.01.01 00:00:00';
input string InpOutputDirectory="QuantForge\\ICMarkets_EST7_2020_present";
// IC Markets server wall time follows New York local time plus seven hours:
// UTC+2 during US standard time and UTC+3 during US daylight time.
input string InpBrokerTimezone="ICMarkets/EST+7";
input int InpChunkDays=31;
input int InpMaximumWaitMinutes=30;
input double InpDefaultCommissionPerLotRoundTurn=7.0;
input string InpZeroCommissionSymbols="US500";
input string InpCommissionCurrency="USD";

string g_symbols[];
ENUM_TIMEFRAMES g_periods[];
string g_period_labels[];
string g_dataset_names[];
string g_status[];
long g_job_bars[];
datetime g_job_first[];
datetime g_job_last[];

int g_job_index=0;
int g_attempts=0;
int g_file=INVALID_HANDLE;
int g_digits=0;
long g_total=0;
datetime g_export_to=0;
datetime g_cursor=0;
datetime g_first_written=0;
datetime g_last_written=0;
string g_data_output="";
string g_data_partial="";
string g_metadata_output="";
string g_metadata_partial="";
bool g_job_open=false;
bool g_complete=false;

string Trimmed(string value)
{
   StringTrimLeft(value);
   StringTrimRight(value);
   return value;
}

string Upper(string value)
{
   StringToUpper(value);
   return value;
}

string JoinPath(const string directory,const string filename)
{
   if(StringLen(directory)==0)
      return filename;
   return directory+"\\"+filename;
}

string SafeName(string value)
{
   StringReplace(value,".","_");
   StringReplace(value,"#","_");
   StringReplace(value,"/","_");
   StringReplace(value,"\\","_");
   StringReplace(value," ","_");
   return value;
}

string TimeOfDay(const datetime value)
{
   MqlDateTime parts;
   TimeToStruct(value,parts);
   return StringFormat("%02d:%02d:%02d",parts.hour,parts.min,parts.sec);
}

string ExportUtc()
{
   string value=TimeToString(TimeGMT(),TIME_DATE|TIME_SECONDS);
   StringReplace(value,".","-");
   StringReplace(value," ","T");
   return value+"Z";
}

bool ParseTimeframe(string value,ENUM_TIMEFRAMES &period,string &label)
{
   value=Upper(Trimmed(value));
   if(value=="M1")
      period=PERIOD_M1;
   else if(value=="H1")
      period=PERIOD_H1;
   else
      return false;
   label=value;
   return true;
}

void AddJob(const string symbol,const ENUM_TIMEFRAMES period,const string label)
{
   const int next=ArraySize(g_symbols)+1;
   ArrayResize(g_symbols,next);
   ArrayResize(g_periods,next);
   ArrayResize(g_period_labels,next);
   ArrayResize(g_dataset_names,next);
   ArrayResize(g_status,next);
   ArrayResize(g_job_bars,next);
   ArrayResize(g_job_first,next);
   ArrayResize(g_job_last,next);
   g_symbols[next-1]=symbol;
   g_periods[next-1]=period;
   g_period_labels[next-1]=label;
   g_dataset_names[next-1]="ICMarketsSC-Demo_"+SafeName(symbol)+"_"+label+"_2020_present";
   g_status[next-1]="pending";
   g_job_bars[next-1]=0;
   g_job_first[next-1]=0;
   g_job_last[next-1]=0;
}

int BuildJobs()
{
   string symbols[];
   string timeframes[];
   const ushort comma=StringGetCharacter(",",0);
   const int symbol_count=StringSplit(InpSymbols,comma,symbols);
   const int timeframe_count=StringSplit(InpTimeframes,comma,timeframes);
   if(symbol_count<=0 || timeframe_count<=0)
      return 0;

   ENUM_TIMEFRAMES periods[];
   string labels[];
   for(int p=0;p<timeframe_count;p++)
   {
      ENUM_TIMEFRAMES period;
      string label;
      if(!ParseTimeframe(timeframes[p],period,label))
      {
         Print("QuantForge batch exporter skipped unsupported timeframe ",timeframes[p]);
         continue;
      }
      const int next=ArraySize(periods)+1;
      ArrayResize(periods,next);
      ArrayResize(labels,next);
      periods[next-1]=period;
      labels[next-1]=label;
   }

   for(int s=0;s<symbol_count;s++)
   {
      const string symbol=Trimmed(symbols[s]);
      if(StringLen(symbol)==0)
         continue;
      if(!SymbolSelect(symbol,true))
      {
         Print("QuantForge batch exporter could not select broker symbol ",symbol,
               ". Error=",GetLastError());
         continue;
      }
      for(int p=0;p<ArraySize(periods);p++)
         AddJob(symbol,periods[p],labels[p]);
   }
   return ArraySize(g_symbols);
}

bool ListedSymbol(const string symbol,const string csv)
{
   string values[];
   const ushort comma=StringGetCharacter(",",0);
   const int count=StringSplit(csv,comma,values);
   const string wanted=Upper(Trimmed(symbol));
   for(int i=0;i<count;i++)
      if(Upper(Trimmed(values[i]))==wanted)
         return true;
   return false;
}

double CommissionForSymbol(const string symbol)
{
   return ListedSymbol(symbol,InpZeroCommissionSymbols)
          ? 0.0
          : InpDefaultCommissionPerLotRoundTurn;
}

void CloseDataFile()
{
   if(g_file!=INVALID_HANDLE)
   {
      FileFlush(g_file);
      FileClose(g_file);
      g_file=INVALID_HANDLE;
   }
}

void DeleteCurrentPartials()
{
   CloseDataFile();
   if(StringLen(g_data_partial)>0)
      FileDelete(g_data_partial,FILE_COMMON);
   if(StringLen(g_metadata_partial)>0)
      FileDelete(g_metadata_partial,FILE_COMMON);
}

void WriteMetadataProperty(const int handle,const string property,const string value)
{
   FileWrite(handle,property,value);
}

bool WriteMetadata(const int job)
{
   const string symbol=g_symbols[job];
   FileDelete(g_metadata_partial,FILE_COMMON);
   const int handle=FileOpen(g_metadata_partial,
                             FILE_WRITE|FILE_CSV|FILE_ANSI|FILE_COMMON,
                             ',',CP_UTF8);
   if(handle==INVALID_HANDLE)
   {
      Print("QuantForge batch metadata export failed. FileOpen error=",GetLastError());
      return false;
   }

   long server_offset=(long)(TimeTradeServer()-TimeGMT());
   server_offset=(server_offset/60)*60;
   const double commission=CommissionForSymbol(symbol);

   FileWrite(handle,"property","value");
   WriteMetadataProperty(handle,"schema_version","1");
   WriteMetadataProperty(handle,"dataset_name",g_dataset_names[job]);
   WriteMetadataProperty(handle,"broker",AccountInfoString(ACCOUNT_COMPANY));
   WriteMetadataProperty(handle,"server",AccountInfoString(ACCOUNT_SERVER));
   WriteMetadataProperty(handle,"terminal_build",IntegerToString((int)TerminalInfoInteger(TERMINAL_BUILD)));
   WriteMetadataProperty(handle,"export_utc",ExportUtc());
   WriteMetadataProperty(handle,"symbol",symbol);
   WriteMetadataProperty(handle,"timeframe",EnumToString(g_periods[job]));
   WriteMetadataProperty(handle,"from_server_time",TimeToString(InpFrom,TIME_DATE|TIME_SECONDS));
   WriteMetadataProperty(handle,"to_server_time",TimeToString(g_export_to,TIME_DATE|TIME_SECONDS));
   WriteMetadataProperty(handle,"first_bar_server_time",TimeToString(g_first_written,TIME_DATE|TIME_SECONDS));
   WriteMetadataProperty(handle,"last_bar_server_time",TimeToString(g_last_written,TIME_DATE|TIME_SECONDS));
   WriteMetadataProperty(handle,"bar_count",IntegerToString((int)g_total));
   WriteMetadataProperty(handle,"broker_timezone",InpBrokerTimezone);
   WriteMetadataProperty(handle,"broker_timezone_rule","America/New_York local wall clock plus 7 hours");
   WriteMetadataProperty(handle,"server_utc_offset_seconds_at_export",IntegerToString((int)server_offset));
   WriteMetadataProperty(handle,"digits",IntegerToString(g_digits));
   WriteMetadataProperty(handle,"point",DoubleToString(SymbolInfoDouble(symbol,SYMBOL_POINT),g_digits+4));
   WriteMetadataProperty(handle,"tick_size",DoubleToString(SymbolInfoDouble(symbol,SYMBOL_TRADE_TICK_SIZE),g_digits+4));
   WriteMetadataProperty(handle,"tick_value",DoubleToString(SymbolInfoDouble(symbol,SYMBOL_TRADE_TICK_VALUE),12));
   WriteMetadataProperty(handle,"contract_size",DoubleToString(SymbolInfoDouble(symbol,SYMBOL_TRADE_CONTRACT_SIZE),8));
   WriteMetadataProperty(handle,"volume_min",DoubleToString(SymbolInfoDouble(symbol,SYMBOL_VOLUME_MIN),8));
   WriteMetadataProperty(handle,"volume_step",DoubleToString(SymbolInfoDouble(symbol,SYMBOL_VOLUME_STEP),8));
   WriteMetadataProperty(handle,"volume_max",DoubleToString(SymbolInfoDouble(symbol,SYMBOL_VOLUME_MAX),8));
   WriteMetadataProperty(handle,"stops_level_points",IntegerToString((int)SymbolInfoInteger(symbol,SYMBOL_TRADE_STOPS_LEVEL)));
   WriteMetadataProperty(handle,"freeze_level_points",IntegerToString((int)SymbolInfoInteger(symbol,SYMBOL_TRADE_FREEZE_LEVEL)));
   WriteMetadataProperty(handle,"filling_mode_flags",IntegerToString((int)SymbolInfoInteger(symbol,SYMBOL_FILLING_MODE)));
   WriteMetadataProperty(handle,"trade_mode",EnumToString((ENUM_SYMBOL_TRADE_MODE)SymbolInfoInteger(symbol,SYMBOL_TRADE_MODE)));
   WriteMetadataProperty(handle,"calculation_mode",EnumToString((ENUM_SYMBOL_CALC_MODE)SymbolInfoInteger(symbol,SYMBOL_TRADE_CALC_MODE)));
   WriteMetadataProperty(handle,"margin_initial",DoubleToString(SymbolInfoDouble(symbol,SYMBOL_MARGIN_INITIAL),12));
   WriteMetadataProperty(handle,"swap_mode",EnumToString((ENUM_SYMBOL_SWAP_MODE)SymbolInfoInteger(symbol,SYMBOL_SWAP_MODE)));
   WriteMetadataProperty(handle,"swap_long",DoubleToString(SymbolInfoDouble(symbol,SYMBOL_SWAP_LONG),12));
   WriteMetadataProperty(handle,"swap_short",DoubleToString(SymbolInfoDouble(symbol,SYMBOL_SWAP_SHORT),12));
   WriteMetadataProperty(handle,"triple_swap_day",EnumToString((ENUM_DAY_OF_WEEK)SymbolInfoInteger(symbol,SYMBOL_SWAP_ROLLOVER3DAYS)));
   WriteMetadataProperty(handle,"swap_multiplier_sunday",DoubleToString(SymbolInfoDouble(symbol,SYMBOL_SWAP_SUNDAY),2));
   WriteMetadataProperty(handle,"swap_multiplier_monday",DoubleToString(SymbolInfoDouble(symbol,SYMBOL_SWAP_MONDAY),2));
   WriteMetadataProperty(handle,"swap_multiplier_tuesday",DoubleToString(SymbolInfoDouble(symbol,SYMBOL_SWAP_TUESDAY),2));
   WriteMetadataProperty(handle,"swap_multiplier_wednesday",DoubleToString(SymbolInfoDouble(symbol,SYMBOL_SWAP_WEDNESDAY),2));
   WriteMetadataProperty(handle,"swap_multiplier_thursday",DoubleToString(SymbolInfoDouble(symbol,SYMBOL_SWAP_THURSDAY),2));
   WriteMetadataProperty(handle,"swap_multiplier_friday",DoubleToString(SymbolInfoDouble(symbol,SYMBOL_SWAP_FRIDAY),2));
   WriteMetadataProperty(handle,"swap_multiplier_saturday",DoubleToString(SymbolInfoDouble(symbol,SYMBOL_SWAP_SATURDAY),2));
   WriteMetadataProperty(handle,"account_currency",AccountInfoString(ACCOUNT_CURRENCY));
   WriteMetadataProperty(handle,"currency_base",SymbolInfoString(symbol,SYMBOL_CURRENCY_BASE));
   WriteMetadataProperty(handle,"currency_profit",SymbolInfoString(symbol,SYMBOL_CURRENCY_PROFIT));
   WriteMetadataProperty(handle,"currency_margin",SymbolInfoString(symbol,SYMBOL_CURRENCY_MARGIN));
   WriteMetadataProperty(handle,"commission_basis","per_lot_round_turn");
   WriteMetadataProperty(handle,"commission_amount",DoubleToString(commission,8));
   WriteMetadataProperty(handle,"commission_currency",InpCommissionCurrency);

   for(int weekday=0;weekday<7;weekday++)
   {
      for(uint index=0;;index++)
      {
         datetime session_from=0;
         datetime session_to=0;
         ResetLastError();
         if(!SymbolInfoSessionTrade(symbol,(ENUM_DAY_OF_WEEK)weekday,index,
                                    session_from,session_to))
            break;
         WriteMetadataProperty(handle,
                               StringFormat("session_%d_%u",weekday,index),
                               StringFormat("%d|%s|%s",weekday,
                                            TimeOfDay(session_from),
                                            TimeOfDay(session_to)));
      }
   }

   FileFlush(handle);
   FileClose(handle);
   ResetLastError();
   if(!FileMove(g_metadata_partial,FILE_COMMON,g_metadata_output,
                FILE_COMMON|FILE_REWRITE))
   {
      Print("QuantForge batch could not publish metadata. Error=",GetLastError());
      FileDelete(g_metadata_partial,FILE_COMMON);
      return false;
   }
   return true;
}

bool OpenCurrentJob()
{
   const string symbol=g_symbols[g_job_index];
   const ENUM_TIMEFRAMES period=g_periods[g_job_index];
   if(!SymbolSelect(symbol,true))
      return false;

   MqlRates seed[];
   ArraySetAsSeries(seed,false);
   ResetLastError();
   CopyRates(symbol,period,0,2,seed);
   const datetime current_open=iTime(symbol,period,0);
   if(current_open<=0 || !(bool)SeriesInfoInteger(symbol,period,SERIES_SYNCHRONIZED))
      return false;

   g_export_to=current_open-1;
   if(g_export_to<=InpFrom)
      return false;
   g_cursor=InpFrom;
   g_attempts=0;
   g_digits=(int)SymbolInfoInteger(symbol,SYMBOL_DIGITS);
   g_total=0;
   g_first_written=0;
   g_last_written=0;
   g_data_output=JoinPath(InpOutputDirectory,g_dataset_names[g_job_index]+".tsv");
   g_data_partial=g_data_output+".partial";
   g_metadata_output=JoinPath(InpOutputDirectory,g_dataset_names[g_job_index]+".metadata.csv");
   g_metadata_partial=g_metadata_output+".partial";
   FileDelete(g_data_partial,FILE_COMMON);
   FileDelete(g_metadata_partial,FILE_COMMON);
   g_file=FileOpen(g_data_partial,
                   FILE_WRITE|FILE_CSV|FILE_ANSI|FILE_COMMON,
                   '\t',CP_UTF8);
   if(g_file==INVALID_HANDLE)
   {
      Print("QuantForge batch data FileOpen failed for ",symbol," ",
            g_period_labels[g_job_index]," error=",GetLastError());
      return false;
   }
   FileWrite(g_file,"<DATE>","<TIME>","<OPEN>","<HIGH>","<LOW>","<CLOSE>",
             "<TICKVOL>","<VOL>","<SPREAD>");
   g_job_open=true;
   g_status[g_job_index]="exporting";
   Print("QuantForge batch started job ",g_job_index+1,"/",ArraySize(g_symbols),
         ": ",symbol," ",g_period_labels[g_job_index]," through ",
         TimeToString(g_export_to));
   return true;
}

void FailCurrentJob(const string reason)
{
   Print("QuantForge batch failed ",g_symbols[g_job_index]," ",
         g_period_labels[g_job_index],": ",reason);
   DeleteCurrentPartials();
   g_status[g_job_index]="failed: "+reason;
   g_job_bars[g_job_index]=g_total;
   g_job_first[g_job_index]=g_first_written;
   g_job_last[g_job_index]=g_last_written;
   g_job_open=false;
   g_job_index++;
   g_attempts=0;
}

void FinishCurrentJob()
{
   CloseDataFile();
   const datetime tolerance=7*24*60*60;
   if(g_total<=0 || g_last_written<g_export_to-tolerance)
   {
      FailCurrentJob("final coverage check failed at "+TimeToString(g_last_written));
      return;
   }
   ResetLastError();
   if(!FileMove(g_data_partial,FILE_COMMON,g_data_output,FILE_COMMON|FILE_REWRITE))
   {
      FailCurrentJob("could not publish data file, error="+IntegerToString(GetLastError()));
      return;
   }
   if(!WriteMetadata(g_job_index))
   {
      FileDelete(g_data_output,FILE_COMMON);
      FailCurrentJob("metadata publication failed");
      return;
   }

   g_status[g_job_index]="complete";
   g_job_bars[g_job_index]=g_total;
   g_job_first[g_job_index]=g_first_written;
   g_job_last[g_job_index]=g_last_written;
   Print("QuantForge batch completed ",g_symbols[g_job_index]," ",
         g_period_labels[g_job_index],": ",g_total," bars");
   g_job_open=false;
   g_job_index++;
   g_attempts=0;
}

void WriteManifest()
{
   const string output=JoinPath(InpOutputDirectory,"_export_manifest.csv");
   const string partial=output+".partial";
   FileDelete(partial,FILE_COMMON);
   const int handle=FileOpen(partial,
                             FILE_WRITE|FILE_CSV|FILE_ANSI|FILE_COMMON,
                             ',',CP_UTF8);
   if(handle==INVALID_HANDLE)
      return;
   FileWrite(handle,"dataset","symbol","timeframe","status","bars","first_bar_server_time",
             "last_bar_server_time","commission_round_turn","commission_currency","timezone_rule");
   for(int i=0;i<ArraySize(g_symbols);i++)
      FileWrite(handle,
                g_dataset_names[i],
                g_symbols[i],
                g_period_labels[i],
                g_status[i],
                g_job_bars[i],
                TimeToString(g_job_first[i],TIME_DATE|TIME_SECONDS),
                TimeToString(g_job_last[i],TIME_DATE|TIME_SECONDS),
                DoubleToString(CommissionForSymbol(g_symbols[i]),2),
                InpCommissionCurrency,
                InpBrokerTimezone);
   FileFlush(handle);
   FileClose(handle);
   FileMove(partial,FILE_COMMON,output,FILE_COMMON|FILE_REWRITE);
}

void FinishBatch()
{
   WriteManifest();
   g_complete=true;
   EventKillTimer();
   int succeeded=0;
   for(int i=0;i<ArraySize(g_status);i++)
      if(g_status[i]=="complete")
         succeeded++;
   Print("QuantForge batch finished: ",succeeded,"/",ArraySize(g_status),
         " jobs completed. Output=Common\\Files\\",InpOutputDirectory);
   Comment("QuantForge batch finished: "+IntegerToString(succeeded)+"/"+
           IntegerToString(ArraySize(g_status))+" completed");
   ExpertRemove();
}

int OnInit()
{
   if((bool)MQLInfoInteger(MQL_TESTER))
   {
      Print("QuantForgeBatchHistoryExporterEA must run on a normal connected chart.");
      return INIT_FAILED;
   }
   if(InpFrom<=0 || InpChunkDays<1 || InpMaximumWaitMinutes<1 ||
      InpDefaultCommissionPerLotRoundTurn<0.0 ||
      StringLen(InpBrokerTimezone)==0)
      return INIT_PARAMETERS_INCORRECT;
   if(StringLen(InpOutputDirectory)>0)
      FolderCreate(InpOutputDirectory,FILE_COMMON);
   if(BuildJobs()<=0)
      return INIT_PARAMETERS_INCORRECT;
   // The work itself is still serialized one bounded chunk at a time. A
   // millisecond timer avoids imposing a full idle second between chunks.
   if(!EventSetMillisecondTimer(100))
      return INIT_FAILED;
   Print("QuantForge batch exporter ready: ",ArraySize(g_symbols),
         " jobs from ",TimeToString(InpFrom),". Non-trading utility.");
   return INIT_SUCCEEDED;
}

void OnTimer()
{
   if(g_job_index>=ArraySize(g_symbols))
   {
      FinishBatch();
      return;
   }

   Comment("QuantForge export "+IntegerToString(g_job_index+1)+"/"+
           IntegerToString(ArraySize(g_symbols))+": "+g_symbols[g_job_index]+" "+
           g_period_labels[g_job_index]);

   if(!g_job_open)
   {
      if(OpenCurrentJob())
         return;
      g_attempts++;
      if(g_attempts==1 || g_attempts%15==0)
         Print("QuantForge batch waiting to initialize ",g_symbols[g_job_index]," ",
               g_period_labels[g_job_index]," attempt=",g_attempts,
               " error=",GetLastError());
      if(g_attempts>=InpMaximumWaitMinutes*60)
         FailCurrentJob("timed out initializing broker history");
      return;
   }

   if(g_cursor>=g_export_to)
   {
      FinishCurrentJob();
      return;
   }

   const datetime proposed_end=g_cursor+(datetime)(InpChunkDays*24*60*60);
   const datetime chunk_end=(proposed_end>g_export_to ? g_export_to : proposed_end);
   MqlRates rates[];
   ArraySetAsSeries(rates,false);
   ResetLastError();
   const int count=CopyRates(g_symbols[g_job_index],g_periods[g_job_index],
                             g_cursor,chunk_end,rates);
   const int error=GetLastError();
   const datetime tolerance=7*24*60*60;
   const bool synchronized=(bool)SeriesInfoInteger(g_symbols[g_job_index],
                                                    g_periods[g_job_index],
                                                    SERIES_SYNCHRONIZED);
   const bool covered=(count>0 && rates[0].time<=g_cursor+tolerance &&
                       rates[count-1].time>=chunk_end-tolerance);
   if(!synchronized || !covered)
   {
      g_attempts++;
      if(g_attempts==1 || g_attempts%15==0)
         Print("QuantForge batch waiting for ",g_symbols[g_job_index]," ",
               g_period_labels[g_job_index]," through ",TimeToString(chunk_end),
               ". Bars=",count," synchronized=",synchronized," error=",error,
               " attempt=",g_attempts);
      if(g_attempts>=InpMaximumWaitMinutes*60)
         FailCurrentJob("timed out waiting for chunk ending "+TimeToString(chunk_end));
      return;
   }

   for(int index=0;index<count;index++)
   {
      if(rates[index].time<=g_last_written)
         continue;
      FileWrite(g_file,
                TimeToString(rates[index].time,TIME_DATE),
                TimeToString(rates[index].time,TIME_MINUTES|TIME_SECONDS),
                DoubleToString(rates[index].open,g_digits),
                DoubleToString(rates[index].high,g_digits),
                DoubleToString(rates[index].low,g_digits),
                DoubleToString(rates[index].close,g_digits),
                rates[index].tick_volume,
                rates[index].real_volume,
                rates[index].spread);
      if(g_first_written<=0)
         g_first_written=rates[index].time;
      g_last_written=rates[index].time;
      g_total++;
   }
   FileFlush(g_file);
   g_cursor=chunk_end;
   g_attempts=0;
   Print("QuantForge batch ",g_symbols[g_job_index]," ",
         g_period_labels[g_job_index]," exported through ",TimeToString(chunk_end),
         ": ",g_total," total bars");
}

void OnDeinit(const int reason)
{
   EventKillTimer();
   if(!g_complete && g_job_open)
      DeleteCurrentPartials();
   if(!g_complete)
      WriteManifest();
   Comment("");
}

void OnTick() {}
