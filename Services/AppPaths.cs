using System.IO;

namespace SceneryAddonsBrowser.Services
{
    public static class AppPaths
    {
        public static string BaseDir => UserStorage.RootPath;

        public static string SceneriesDir =>
            Path.Combine(BaseDir, "sceneries");

        public static string GetScenarioDir(string scenarioId)
        {
            var dir = Path.Combine(SceneriesDir, scenarioId);
            Directory.CreateDirectory(dir);
            return dir;
        }

        public static string GetScenarioDataDir(string scenarioId)
        {
            var dir = Path.Combine(GetScenarioDir(scenarioId), "data");
            Directory.CreateDirectory(dir);
            return dir;
        }

        public static string GetSceneriesRoot()
        {
            Directory.CreateDirectory(SceneriesDir);
            return SceneriesDir;
        }
    }
}
