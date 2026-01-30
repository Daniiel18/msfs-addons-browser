using System;
using System.IO;

namespace SceneryAddonsBrowser.Logging
{
    public static class AppLogger
    {
        private static readonly string LogDirectory =
            Path.Combine(
                Environment.GetFolderPath(Environment.SpecialFolder.MyDocuments),
                "SceneryAddonsBrowser",
                "logs"
            );

        private static readonly string LogFilePath =
            Path.Combine(LogDirectory, "app.log");

        public static void Log(string message)
        {
            try
            {
                if (!Directory.Exists(LogDirectory))
                    Directory.CreateDirectory(LogDirectory);

                string line =
                    $"[{DateTime.Now:yyyy-MM-dd HH:mm:ss}] {message}";

                File.AppendAllText(LogFilePath, line + Environment.NewLine);
            }
            catch
            {
                // Nunca romper la app por logging
            }
        }

        public static void LogError(string message, Exception ex)
        {
            Log($"ERROR: {message}");
            Log($"EXCEPTION: {ex.Message}");
        }
    }
}
