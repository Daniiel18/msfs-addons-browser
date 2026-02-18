using SceneryAddonsBrowser.Logging;
using SharpCompress.Archives;
using SharpCompress.Common;
using System.Diagnostics;
using System.IO;



namespace SceneryAddonsBrowser.Services
{
    public class InstallerService
    {
        private void CopyDirectory(string source, string destination)
        {
            Directory.CreateDirectory(destination);

            foreach (var file in Directory.GetFiles(source))
            {
                string targetFile = Path.Combine(destination, Path.GetFileName(file));
                File.Copy(file, targetFile, true);
            }

            foreach (var dir in Directory.GetDirectories(source))
            {
                string targetDir = Path.Combine(destination, Path.GetFileName(dir));
                CopyDirectory(dir, targetDir);
            }
        }

        public void InstallFromExtracted(string extractedPath, string communityPath)
        {
            var packages = FindMsfsPackages(extractedPath);

            if (packages.Count == 0)
                throw new Exception("No MSFS scenery packages found to install.");

            foreach (var packageDir in packages)
            {
                var folderName = Path.GetFileName(packageDir);
                var targetDir = Path.Combine(communityPath, folderName);

                AppLogger.Log($"Installing {folderName} → Community");

                if (Directory.Exists(targetDir))
                    Directory.Delete(targetDir, true);

                CopyDirectory(packageDir, targetDir);
            }

            AppLogger.Log("All packages installed successfully.");
        }

        public string ExtractPackage(string packagePath)
        {
            if (!File.Exists(packagePath))
                throw new FileNotFoundException("Package not found.", packagePath);

            var extractDir = Path.Combine(
                Path.GetTempPath(),
                "SceneryAddonsBrowser",
                "extract",
                Guid.NewGuid().ToString());

            Directory.CreateDirectory(extractDir);

            AppLogger.Log($"Extracting package: {Path.GetFileName(packagePath)}");
            AppLogger.Log($"Extract target: {extractDir}");

            try
            {
                using var archive = ArchiveFactory.Open(packagePath);

                foreach (var entry in archive.Entries)
                {
                    if (!entry.IsDirectory)
                    {
                        entry.WriteToDirectory(
                            extractDir,
                            new ExtractionOptions
                            {
                                ExtractFullPath = true,
                                Overwrite = true
                            });
                    }
                }

                AppLogger.Log("Extraction completed.");
                return extractDir;
            }
            catch (Exception ex) when (
                ex is IndexOutOfRangeException ||
                ex.Message.Contains("RAR", StringComparison.OrdinalIgnoreCase))
            {
                AppLogger.LogError(
                    "[INSTALL] Automatic extraction failed. Manual extraction required.",
                    ex);

                AppLogger.Log("[INSTALL] This RAR archive is not fully compatible with SharpCompress (RAR5).");

                Process.Start(new ProcessStartInfo
                {
                    FileName = extractDir,
                    UseShellExecute = true
                });

                Process.Start(new ProcessStartInfo
                {
                    FileName = packagePath,
                    UseShellExecute = true
                });

                throw new InvalidOperationException(
                    "Automatic extraction failed. Please extract the archive manually and retry installation.");
            }
        }

        public List<string> FindMsfsPackages(string extractedPath)
        {
            var result = new List<string>();

            foreach (var dir in Directory.GetDirectories(extractedPath))
            {
                var manifest = Path.Combine(dir, "manifest.json");
                var layout = Path.Combine(dir, "layout.json");

                if (File.Exists(manifest) && File.Exists(layout))
                {
                    AppLogger.Log($"MSFS package found: {Path.GetFileName(dir)}");
                    result.Add(dir);
                }
            }

            if (result.Count == 0)
                AppLogger.Log("No valid MSFS packages found.");

            return result;
        }

    }
}
