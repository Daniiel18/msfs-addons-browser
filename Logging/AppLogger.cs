using System.IO;

namespace SceneryAddonsBrowser.Logging
{
    public static class AppLogger
    {
        private static string LogDirectory =>
            Path.Combine(UserStorage.RootPath, "logs");

        private static string LogFilePath =>
            Path.Combine(LogDirectory, "app.log");

        public static void Log(string message)
        {
            try
            {
                Directory.CreateDirectory(LogDirectory);

                string line = $"[{DateTime.Now:yyyy-MM-dd HH:mm:ss}] {message}";
                File.AppendAllText(LogFilePath, line + Environment.NewLine);
            }
            catch { }
        }

        public static void LogError(string message, Exception ex)
        {
            Log($"ERROR: {message}");
            Log($"EXCEPTION: {ex}");
        }
    }
}
