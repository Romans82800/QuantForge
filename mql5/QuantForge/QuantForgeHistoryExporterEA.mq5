#property strict
#property version   "1.00"
#property description "Non-trading, chunked OHLCV and broker-metadata exporter for QuantForge."

input string InpSymbol="EURUSD";
input ENUM_TIMEFRAMES InpTimeframe=PERIOD_M1;
input datetime InpFrom=D'2015.01.01 00:00';
input datetime InpTo=D'2026.07.01 00:00';
input string InpDatasetName="ICMarketsSC-Demo_EURUSD_M1_2015_2026";
input string InpOutputDirectory="QuantForge";
// MT5 bar timestamps are broker-server wall time. This must be an IANA timezone
// understood by QuantForge, for example Europe/Helsinki. Never guess it.
input string InpBrokerTimezone="REQUIRED";
input int InpChunkDays=31;
input int InpMaximumWaitMinutes=30;
// MT5 does not expose account commission through SymbolInfo*. Record the
// explicitly verified round-turn value, or leave -1 and complete it later.
input double InpCommissionPerLotRoundTurn=-1.0;
input string InpCommissionCurrency="";

string g_data_partial="";
string g_data_output="";
string g_metadata_partial="";
string g_metadata_output="";
int g_file=INVALID_HANDLE;
int g_digits=0;
int g_attempts=0;
long g_total=0;
datetime g_cursor=0;
datetime g_last_written=0;
bool g_complete=false;

string JoinPath(const string directory,const string filename)
{
   if(StringLen(directory)==0)
      return filename;
   return directory+"\\"+filename;
}

bool SafeDatasetName(const string value)
{
   return StringLen(value)>0
          && StringFind(value,"\\")<0
          && StringFind(value,"/")<0
          && StringFind(value,":")<0;
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

void CloseDataFile()
{
   if(g_file!=INVALID_HANDLE)
   {
      FileFlush(g_file);
      FileClose(g_file);
      g_file=INVALID_HANDLE;
   }
}

void StopExport(const string message)
{
   Print(message);
   EventKillTimer();
   CloseDataFile();
   if(StringLen(g_data_partial)>0)
      FileDelete(g_data_partial,FILE_COMMON);
   if(StringLen(g_metadata_partial)>0)
      FileDelete(g_metadata_partial,FILE_COMMON);
   ExpertRemove();
}

void WriteMetadataProperty(const int handle,const string property,const string value)
{
   FileWrite(handle,property,value);
}

bool WriteMetadata()
{
   FileDelete(g_metadata_partial,FILE_COMMON);
   const int handle=FileOpen(g_metadata_partial,
                             FILE_WRITE|FILE_CSV|FILE_ANSI|FILE_COMMON,
                             ',',CP_UTF8);
   if(handle==INVALID_HANDLE)
   {
      Print("QuantForge metadata export failed. FileOpen error=",GetLastError());
      return false;
   }

   const string commission_currency=StringLen(InpCommissionCurrency)>0
                                    ? InpCommissionCurrency
                                    : AccountInfoString(ACCOUNT_CURRENCY);
   long server_offset=(long)(TimeTradeServer()-TimeGMT());
   server_offset=(server_offset/60)*60;

   FileWrite(handle,"property","value");
   WriteMetadataProperty(handle,"schema_version","1");
   WriteMetadataProperty(handle,"dataset_name",InpDatasetName);
   WriteMetadataProperty(handle,"broker",AccountInfoString(ACCOUNT_COMPANY));
   WriteMetadataProperty(handle,"server",AccountInfoString(ACCOUNT_SERVER));
   WriteMetadataProperty(handle,"terminal_build",IntegerToString((int)TerminalInfoInteger(TERMINAL_BUILD)));
   WriteMetadataProperty(handle,"export_utc",ExportUtc());
   WriteMetadataProperty(handle,"symbol",InpSymbol);
   WriteMetadataProperty(handle,"timeframe",EnumToString(InpTimeframe));
   WriteMetadataProperty(handle,"from_server_time",TimeToString(InpFrom,TIME_DATE|TIME_SECONDS));
   WriteMetadataProperty(handle,"to_server_time",TimeToString(InpTo,TIME_DATE|TIME_SECONDS));
   WriteMetadataProperty(handle,"last_bar_server_time",TimeToString(g_last_written,TIME_DATE|TIME_SECONDS));
   WriteMetadataProperty(handle,"bar_count",IntegerToString(g_total));
   WriteMetadataProperty(handle,"broker_timezone",InpBrokerTimezone);
   WriteMetadataProperty(handle,"server_utc_offset_seconds_at_export",IntegerToString(server_offset));
   WriteMetadataProperty(handle,"digits",IntegerToString(g_digits));
   WriteMetadataProperty(handle,"point",DoubleToString(SymbolInfoDouble(InpSymbol,SYMBOL_POINT),g_digits+4));
   WriteMetadataProperty(handle,"tick_size",DoubleToString(SymbolInfoDouble(InpSymbol,SYMBOL_TRADE_TICK_SIZE),g_digits+4));
   WriteMetadataProperty(handle,"tick_value",DoubleToString(SymbolInfoDouble(InpSymbol,SYMBOL_TRADE_TICK_VALUE),12));
   WriteMetadataProperty(handle,"contract_size",DoubleToString(SymbolInfoDouble(InpSymbol,SYMBOL_TRADE_CONTRACT_SIZE),8));
   WriteMetadataProperty(handle,"volume_min",DoubleToString(SymbolInfoDouble(InpSymbol,SYMBOL_VOLUME_MIN),8));
   WriteMetadataProperty(handle,"volume_step",DoubleToString(SymbolInfoDouble(InpSymbol,SYMBOL_VOLUME_STEP),8));
   WriteMetadataProperty(handle,"volume_max",DoubleToString(SymbolInfoDouble(InpSymbol,SYMBOL_VOLUME_MAX),8));
   WriteMetadataProperty(handle,"stops_level_points",IntegerToString((int)SymbolInfoInteger(InpSymbol,SYMBOL_TRADE_STOPS_LEVEL)));
   WriteMetadataProperty(handle,"freeze_level_points",IntegerToString((int)SymbolInfoInteger(InpSymbol,SYMBOL_TRADE_FREEZE_LEVEL)));
   WriteMetadataProperty(handle,"filling_mode_flags",IntegerToString((int)SymbolInfoInteger(InpSymbol,SYMBOL_FILLING_MODE)));
   WriteMetadataProperty(handle,"trade_mode",EnumToString((ENUM_SYMBOL_TRADE_MODE)SymbolInfoInteger(InpSymbol,SYMBOL_TRADE_MODE)));
   WriteMetadataProperty(handle,"calculation_mode",EnumToString((ENUM_SYMBOL_CALC_MODE)SymbolInfoInteger(InpSymbol,SYMBOL_TRADE_CALC_MODE)));
   WriteMetadataProperty(handle,"margin_initial",DoubleToString(SymbolInfoDouble(InpSymbol,SYMBOL_MARGIN_INITIAL),12));
   WriteMetadataProperty(handle,"swap_mode",EnumToString((ENUM_SYMBOL_SWAP_MODE)SymbolInfoInteger(InpSymbol,SYMBOL_SWAP_MODE)));
   WriteMetadataProperty(handle,"swap_long",DoubleToString(SymbolInfoDouble(InpSymbol,SYMBOL_SWAP_LONG),12));
   WriteMetadataProperty(handle,"swap_short",DoubleToString(SymbolInfoDouble(InpSymbol,SYMBOL_SWAP_SHORT),12));
   WriteMetadataProperty(handle,"triple_swap_day",EnumToString((ENUM_DAY_OF_WEEK)SymbolInfoInteger(InpSymbol,SYMBOL_SWAP_ROLLOVER3DAYS)));
   WriteMetadataProperty(handle,"swap_multiplier_sunday",DoubleToString(SymbolInfoDouble(InpSymbol,SYMBOL_SWAP_SUNDAY),2));
   WriteMetadataProperty(handle,"swap_multiplier_monday",DoubleToString(SymbolInfoDouble(InpSymbol,SYMBOL_SWAP_MONDAY),2));
   WriteMetadataProperty(handle,"swap_multiplier_tuesday",DoubleToString(SymbolInfoDouble(InpSymbol,SYMBOL_SWAP_TUESDAY),2));
   WriteMetadataProperty(handle,"swap_multiplier_wednesday",DoubleToString(SymbolInfoDouble(InpSymbol,SYMBOL_SWAP_WEDNESDAY),2));
   WriteMetadataProperty(handle,"swap_multiplier_thursday",DoubleToString(SymbolInfoDouble(InpSymbol,SYMBOL_SWAP_THURSDAY),2));
   WriteMetadataProperty(handle,"swap_multiplier_friday",DoubleToString(SymbolInfoDouble(InpSymbol,SYMBOL_SWAP_FRIDAY),2));
   WriteMetadataProperty(handle,"swap_multiplier_saturday",DoubleToString(SymbolInfoDouble(InpSymbol,SYMBOL_SWAP_SATURDAY),2));
   WriteMetadataProperty(handle,"account_currency",AccountInfoString(ACCOUNT_CURRENCY));
   WriteMetadataProperty(handle,"currency_base",SymbolInfoString(InpSymbol,SYMBOL_CURRENCY_BASE));
   WriteMetadataProperty(handle,"currency_profit",SymbolInfoString(InpSymbol,SYMBOL_CURRENCY_PROFIT));
   WriteMetadataProperty(handle,"currency_margin",SymbolInfoString(InpSymbol,SYMBOL_CURRENCY_MARGIN));
   WriteMetadataProperty(handle,"commission_basis","per_lot_round_turn");
   WriteMetadataProperty(handle,"commission_amount",
                         InpCommissionPerLotRoundTurn>=0.0
                         ? DoubleToString(InpCommissionPerLotRoundTurn,8)
                         : "UNSPECIFIED");
   WriteMetadataProperty(handle,"commission_currency",commission_currency);

   for(int weekday=0;weekday<7;weekday++)
   {
      for(uint index=0;;index++)
      {
         datetime session_from=0;
         datetime session_to=0;
         ResetLastError();
         if(!SymbolInfoSessionTrade(InpSymbol,(ENUM_DAY_OF_WEEK)weekday,index,
                                    session_from,session_to))
            break;
         const string key=StringFormat("session_%d_%u",weekday,index);
         const string value=StringFormat("%d|%s|%s",weekday,
                                         TimeOfDay(session_from),
                                         TimeOfDay(session_to));
         WriteMetadataProperty(handle,key,value);
      }
   }

   FileFlush(handle);
   FileClose(handle);
   ResetLastError();
   if(!FileMove(g_metadata_partial,FILE_COMMON,g_metadata_output,
                FILE_COMMON|FILE_REWRITE))
   {
      Print("QuantForge could not publish metadata. Error=",GetLastError());
      FileDelete(g_metadata_partial,FILE_COMMON);
      return false;
   }
   return true;
}

void FinishExport()
{
   EventKillTimer();
   CloseDataFile();
   const datetime tolerance=7*24*60*60;
   if(g_total<=0 || g_last_written<InpTo-tolerance)
   {
      StopExport("QuantForge export failed final coverage check. Last bar="+
                 TimeToString(g_last_written));
      return;
   }
   ResetLastError();
   if(!FileMove(g_data_partial,FILE_COMMON,g_data_output,FILE_COMMON|FILE_REWRITE))
   {
      StopExport("QuantForge could not publish completed data file. Error="+
                 IntegerToString(GetLastError()));
      return;
   }
   if(!WriteMetadata())
   {
      FileDelete(g_data_output,FILE_COMMON);
      StopExport("QuantForge removed the data file because metadata publication failed.");
      return;
   }
   g_complete=true;
   Print("QuantForge exported ",g_total," bars to Common\\Files\\",g_data_output,
         " with metadata ",g_metadata_output);
   ExpertRemove();
}

int OnInit()
{
   if((bool)MQLInfoInteger(MQL_TESTER))
   {
      Print("QuantForgeHistoryExporterEA is a non-trading data utility. Attach it to a normal connected chart.");
      return INIT_FAILED;
   }
   if(InpFrom>=InpTo || InpChunkDays<1 || InpMaximumWaitMinutes<1)
      return INIT_PARAMETERS_INCORRECT;
   if(InpBrokerTimezone=="REQUIRED" || StringLen(InpBrokerTimezone)==0)
   {
      Print("QuantForge requires an explicit broker IANA timezone before export.");
      return INIT_PARAMETERS_INCORRECT;
   }
   if(!SafeDatasetName(InpDatasetName))
   {
      Print("QuantForge dataset name cannot be empty or contain path separators.");
      return INIT_PARAMETERS_INCORRECT;
   }
   if(!SymbolSelect(InpSymbol,true))
   {
      Print("QuantForge could not select ",InpSymbol,". Error=",GetLastError());
      return INIT_FAILED;
   }

   if(StringLen(InpOutputDirectory)>0)
      FolderCreate(InpOutputDirectory,FILE_COMMON);
   g_digits=(int)SymbolInfoInteger(InpSymbol,SYMBOL_DIGITS);
   g_cursor=InpFrom;
   g_data_output=JoinPath(InpOutputDirectory,InpDatasetName+".tsv");
   g_data_partial=g_data_output+".partial";
   g_metadata_output=JoinPath(InpOutputDirectory,InpDatasetName+".metadata.csv");
   g_metadata_partial=g_metadata_output+".partial";
   FileDelete(g_data_partial,FILE_COMMON);
   FileDelete(g_metadata_partial,FILE_COMMON);

   g_file=FileOpen(g_data_partial,
                   FILE_WRITE|FILE_CSV|FILE_ANSI|FILE_COMMON,
                   '\t',CP_UTF8);
   if(g_file==INVALID_HANDLE)
   {
      Print("QuantForge data export failed. FileOpen error=",GetLastError());
      return INIT_FAILED;
   }
   FileWrite(g_file,"<DATE>","<TIME>","<OPEN>","<HIGH>","<LOW>","<CLOSE>",
             "<TICKVOL>","<VOL>","<SPREAD>");
   EventSetTimer(1);
   Print("QuantForge synchronising ",InpSymbol," ",EnumToString(InpTimeframe),
         " from ",TimeToString(InpFrom)," to ",TimeToString(InpTo));
   return INIT_SUCCEEDED;
}

void OnTimer()
{
   if(g_cursor>=InpTo)
   {
      FinishExport();
      return;
   }

   const datetime proposed_end=g_cursor+(datetime)(InpChunkDays*24*60*60);
   const datetime chunk_end=(proposed_end>InpTo ? InpTo : proposed_end);
   MqlRates rates[];
   ArraySetAsSeries(rates,false);
   ResetLastError();
   const int count=CopyRates(InpSymbol,InpTimeframe,g_cursor,chunk_end,rates);
   const datetime tolerance=7*24*60*60;
   const bool synchronized=(bool)SeriesInfoInteger(InpSymbol,InpTimeframe,SERIES_SYNCHRONIZED);
   const bool covered=(count>0
                       && rates[0].time<=g_cursor+tolerance
                       && rates[count-1].time>=chunk_end-tolerance);
   if(!synchronized || !covered)
   {
      g_attempts++;
      if(g_attempts==1 || g_attempts%15==0)
         Print("QuantForge waiting for chunk ending ",TimeToString(chunk_end),
               ". Bars=",count," synchronized=",synchronized,
               " error=",GetLastError());
      if(g_attempts>=InpMaximumWaitMinutes*60)
         StopExport("QuantForge timed out waiting for history ending "+
                    TimeToString(chunk_end)+". Open the symbol/timeframe chart, request more history, then reattach the exporter.");
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
      g_last_written=rates[index].time;
      g_total++;
   }
   FileFlush(g_file);
   Print("QuantForge exported through ",TimeToString(chunk_end),
         ": ",count," bars in chunk, ",g_total," total");
   g_cursor=chunk_end;
   g_attempts=0;
}

void OnDeinit(const int reason)
{
   EventKillTimer();
   CloseDataFile();
   if(!g_complete)
   {
      if(StringLen(g_data_partial)>0)
         FileDelete(g_data_partial,FILE_COMMON);
      if(StringLen(g_metadata_partial)>0)
         FileDelete(g_metadata_partial,FILE_COMMON);
   }
}

void OnTick() {}
