using SceneryAddonsBrowser.Logging;
using SceneryAddonsBrowser.Models;
using SceneryAddonsBrowser.Services;
using SceneryAddonsBrowser.Views;
using System.ComponentModel;
using System.Diagnostics;
using System.Net.Http;
using System.Text.RegularExpressions;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;
using Velopack;
using System.Reflection;
using Application = System.Windows.Application;
using Brush = System.Windows.Media.Brush;
using Cursors = System.Windows.Input.Cursors;
using MessageBox = System.Windows.MessageBox;
using MouseEventArgs = System.Windows.Input.MouseEventArgs;


namespace SceneryAddonsBrowser
{
    public partial class MainWindow : Window
    {
        private readonly SearchService _searchService;
        private readonly DownloadService _downloadService;
        private readonly DownloadStatus _downloadStatus = new();
        private readonly HistoryService _historyService = new();
        private readonly GsxProfileService _gsxService;
        private PendingUpdate? _pendingUpdate;
        private readonly UpdateService _updateService = new();
        private readonly SettingsService _settingsService = new();
        private UpdateInfo? _pendingUpdates;



        private bool _isDownloadAllowed = true;
        public bool IsDownloadAllowed
        {
            get => _isDownloadAllowed;
            set
            {
                _isDownloadAllowed = value;
                Dispatcher.Invoke(() =>
                {
                    ResultsListView.Items.Refresh();
                });
            }
        }
        private string? _lastSearchedIcao;
        private int _lastGsxCount;
        public MainWindow()
        {
            InitializeComponent();

            _searchService = new SearchService();
            _downloadService = new DownloadService();
            _gsxService = new GsxProfileService();

            _downloadService.StateChanged += OnDownloadStateChanged;

            _downloadService.QueueChanged += UpdateQueueUi;

            Loaded += MainWindow_Loaded;
        }

        private void UpdateQueueUi()
        {
            Dispatcher.Invoke(() =>
            {
                int queued = _downloadService.QueueCount;
                bool downloading = _downloadService.IsDownloading;

                int current = downloading ? 1 : 0;
                int total = queued + current;

                if (total <= 1)
                {
                    QueueStatusTextBlock.Visibility = Visibility.Collapsed;
                    return;
                }

                QueueStatusTextBlock.Text = $"{current} / {total} in queue";
                QueueStatusTextBlock.Visibility = Visibility.Visible;
            });
        }


        // ================= AUTOFOCUS =================
        private async void MainWindow_Loaded(object sender, RoutedEventArgs e)
        {
            var pending = PendingUpdateStore.PendingUpdate;

            // DEV MODE
            if (DevFlags.ForceUpdateDialog && pending == null)
            {
                Hide();
                ShowDevUpdateDialog();
                Show();
                return;
            }

            if (pending == null)
                return;

            var settings = _settingsService.Load();

            string newVersion =
                pending.UpdateInfo.TargetFullRelease.Version.ToString();

            if (settings.IgnoredUpdateVersion == newVersion)
            {
                ShowUpdateIndicator(newVersion);
                return;
            }

            Hide();
            ShowUpdateDialog(pending);
            Show();
        }


        private void ShowUpdateDialog(PendingUpdate pending)
        {
            var dialog = new UpdateDialog(
                pending.CurrentVersion,
                pending.UpdateInfo.TargetFullRelease.Version.ToString(),
                pending.Changelog
            )
            {
                Owner = this
            };

            bool? result = dialog.ShowDialog();

            if (result == true && dialog.ShouldUpdate)
            {
                _ = ApplyPendingUpdateAsync(pending);
            }
        }

        private async Task ApplyPendingUpdateAsync(PendingUpdate pending)
        {
            try
            {
                StatusTextBlock.Text = "Applying update…";
                IsEnabled = false;

                await _updateService.ApplyUpdateAsync(pending.UpdateInfo);

                Application.Current.Shutdown();
            }
            catch (Exception ex)
            {
                AppLogger.LogError("[UPDATE] Failed to apply update", ex);
                IsEnabled = true;
            }
        }

        private void ShowDevUpdateDialog()
        {
            var dialog = new UpdateDialog(
                "3.4.0",
                "3.4.X (DEV)",
                new[]
                {
            "• DEV MODE active",
            "• Update dialog test",
            "• No update will be applied"
                })
            {
                Owner = this
            };

            dialog.ShowDialog();
        }

        private void ShowUpdateIndicator(string version)
        {
            UpdateIndicatorTextBlock.Text = $"Update available (v{version})";
            UpdateIndicatorTextBlock.Visibility = Visibility.Visible;

            AppLogger.Log($"[UPDATE] Update indicator shown for version {version}");
        }

        private async void UpdateIndicator_Click(object sender, MouseButtonEventArgs e)
        {
            if (_pendingUpdate == null)
                return;

            AppLogger.Log("[UPDATE] User clicked update indicator");

            this.Hide();

            var settings = _settingsService.Load();

            var changelog = new List<string>
    {
        _pendingUpdates.TargetFullRelease.NotesHTML ?? "No changelog provided."
    };

            var dialog = new Views.UpdateDialog(
                typeof(App).Assembly.GetName().Version?.ToString() ?? "Unknown",
                _pendingUpdates.TargetFullRelease.Version.ToString(),
                changelog
            );

            bool? result = dialog.ShowDialog();

            if (result == true && dialog.ShouldUpdate)
            {
                await _updateService.ApplyUpdateAsync(_pendingUpdates);
                return;
            }

            settings.IgnoredUpdateVersion =
                _pendingUpdates.TargetFullRelease.Version.ToString();

            _settingsService.Save(settings);

            this.Show();
        }

        private void UpdateIndicator_MouseEnter(object sender, MouseEventArgs e)
        {
            UpdateIndicatorTextBlock.TextDecorations = TextDecorations.Underline;
        }

        private void UpdateIndicator_MouseLeave(object sender, MouseEventArgs e)
        {
            UpdateIndicatorTextBlock.TextDecorations = null;
        }

        private void OnDownloadStateChanged(DownloadState state)
        {
            Dispatcher.Invoke(() =>
            {
                // --- RESET ---
                PauseButton.Visibility = Visibility.Collapsed;
                ResumeButton.Visibility = Visibility.Collapsed;
                CancelButton.Visibility = Visibility.Collapsed;

                DownloadProgressBar.Visibility = Visibility.Collapsed;

                // --- LOCK SEARCH + DOWNLOAD ---
                bool lockUi = state == DownloadState.ResolvingMagnet;

                IcaoTextBox.IsEnabled = !lockUi;
                SearchButton.IsEnabled = !lockUi;
                IsDownloadAllowed = !lockUi;

                if (state is DownloadState.Downloading or DownloadState.Paused)
                {
                    UpdateQueueUi();
                }
                else
                {
                    QueueStatusTextBlock.Visibility = Visibility.Collapsed;
                }


                switch (state)
                {
                    case DownloadState.ResolvingMagnet:
                        StatusTextBlock.Text = "Resolving magnet…";
                        DownloadProgressBar.Visibility = Visibility.Visible;
                        DownloadProgressBar.Foreground =
                            (Brush)FindResource("DownloadYellow");
                        break;

                    case DownloadState.Downloading:
                        StatusTextBlock.Text = "Downloading…";
                        DownloadProgressBar.Visibility = Visibility.Visible;
                        DownloadProgressBar.Foreground =
                            (Brush)FindResource("DownloadGreen");

                        PauseButton.Visibility = Visibility.Visible;
                        CancelButton.Visibility = Visibility.Visible;
                        break;

                    case DownloadState.Paused:
                        StatusTextBlock.Text = "Download paused";
                        DownloadProgressBar.Visibility = Visibility.Visible;
                        DownloadProgressBar.Foreground =
                            (Brush)FindResource("DownloadGray");

                        ResumeButton.Visibility = Visibility.Visible;
                        CancelButton.Visibility = Visibility.Visible;
                        break;

                    case DownloadState.Installing:
                        StatusTextBlock.Text = "Installing scenery…";
                        DownloadProgressBar.Visibility = Visibility.Visible;
                        DownloadProgressBar.Foreground =
                            (Brush)FindResource("DownloadGreen");
                        break;

                    case DownloadState.Completed:
                        StatusTextBlock.Text = "Completed";
                        DownloadProgressBar.Value = 0;
                        DownloadProgressBar.Visibility = Visibility.Collapsed;
                        break;

                    case DownloadState.Cancelled:
                    case DownloadState.Error:
                        StatusTextBlock.Text = "Ready";
                        DownloadProgressBar.Value = 0;
                        DownloadProgressBar.Visibility = Visibility.Collapsed;
                        break;
                }
            });
        }

        private void IcaoTextBox_Loaded(object sender, RoutedEventArgs e)
        {
            IcaoTextBox.Focus();
            Keyboard.Focus(IcaoTextBox);
        }

        // ================= ENTER = SEARCH =================
        private async void IcaoTextBox_KeyDown(object sender, System.Windows.Input.KeyEventArgs e)
        {
            if (e.Key == Key.Enter)
            {
                await PerformSearchAsync();
                e.Handled = true;
            }
        }

        private async void SearchButton_Click(object sender, RoutedEventArgs e)
        {
            await PerformSearchAsync();
        }

        private async Task PerformSearchAsync()
        {
            var icao = IcaoTextBox.Text?.Trim();

            if (string.IsNullOrWhiteSpace(icao))
            {
                StatusTextBlock.Text = "Please enter an ICAO code.";
                return;
            }

            StatusTextBlock.Text = "Searching on SceneryAddons.org...";
            ResultsListView.ItemsSource = null;

            var results = await _searchService.SearchByIcaoAsync(icao);

            AppLogger.Log($"UI received {results.Count} scenarios.");

            if (results != null && _gsxService != null)
            {
                foreach (var scenario in results)
                {
                    if (scenario == null)
                        continue;

                    _ = _gsxService.CheckGsxProfileAsync(scenario);
                }
            }

            var gsxService = new GsxProfileService();

            foreach (var scenario in results)
            {
                _ = _gsxService.CheckGsxProfileAsync(scenario);
            }

            ResultsListView.ItemsSource = results;
            StatusTextBlock.Text = $"{results.Count} scenaries found.";

            AppLogger.Log($"[GSX] Triggering GSX lookup after search for ICAO: {icao}");
            await UpdateGsxStatusAsync(icao);

        }

        // ================= DOWNLOAD =================
        private async void DownloadButton_Click(object sender, RoutedEventArgs e)
        {
            if (sender is not FrameworkElement element)
                return;

            if (element.DataContext is not Scenario scenario)
                return;

            var dialog = new DownloadDialog(scenario)
            {
                Owner = this
            };

            var communityService = new CommunityFolderService();

            bool? result = dialog.ShowDialog();
            if (result == true && dialog.SelectedMethod != null)
            {
                try
                {
                    ShowProgress($"Starting {dialog.SelectedMethod.Name}...");

                    await _downloadService.StartDownloadAsync(
                    dialog.SelectedMethod,
                    (text, value) =>
                    {
                        Dispatcher.Invoke(() =>
                    {
                        DownloadProgressBar.Value = value;
                        StatusTextBlock.Text = text;
                    });
                    });
                }
                catch
                {
                }
            }
        }

        private async Task UpdateGsxStatusAsync(string icao)
        {
            if (string.IsNullOrWhiteSpace(icao))
                return;

            _lastSearchedIcao = icao.ToUpperInvariant();
            _lastGsxCount = 0;

            var url = $"https://flightsim.to/others/gsx-pro/search/{icao.ToLowerInvariant()}";

            AppLogger.Log($"[GSX] Starting GSX lookup for ICAO: {_lastSearchedIcao}");
            AppLogger.Log($"[GSX] URL: {url}");

            try
            {
                using var http = new HttpClient();
                http.DefaultRequestHeaders.UserAgent.ParseAdd(
                    "Mozilla/5.0 (Windows NT 10.0; Win64; x64)");

                AppLogger.Log("[GSX] Downloading GSX search page...");
                var html = await http.GetStringAsync(url);

                AppLogger.Log($"[GSX] HTML downloaded. Length = {html.Length}");

                int count = Regex.Matches(
                    html,
                    "tiles-box flex-item new-tile",
                    RegexOptions.IgnoreCase
                ).Count;

                _lastGsxCount = count;

                AppLogger.Log($"[GSX] Profiles found: {count}");

                if (count > 0)
                {
                    GsxStatusTextBlock.Text =
                    $"● GSX Profiles available ({count}) — View on flightsim.to";

                    GsxStatusTextBlock.Foreground =
                        new BrushConverter().ConvertFrom("#FF4FC3F7") as Brush;

                    GsxStatusTextBlock.Cursor = Cursors.Hand;
                    GsxStatusTextBlock.Visibility = Visibility.Visible;

                }
                else
                {
                    GsxStatusTextBlock.Text =
                    "● No GSX profiles found for this airport";

                    GsxStatusTextBlock.Foreground =
                        new BrushConverter().ConvertFrom("#FF777777") as Brush;

                    GsxStatusTextBlock.Cursor = Cursors.Arrow;
                    GsxStatusTextBlock.Visibility = Visibility.Visible;

                }

                GsxStatusTextBlock.Visibility = Visibility.Visible;
                AppLogger.Log("[GSX] Status text updated successfully");
            }
            catch (HttpRequestException ex)
            {
                AppLogger.LogError("[GSX] HTTP error while fetching GSX profiles", ex);

                GsxStatusTextBlock.Text =
                    $"GSX Profiles: Unable to check profiles for {_lastSearchedIcao}";
                GsxStatusTextBlock.Visibility = Visibility.Visible;
            }
            catch (Exception ex)
            {
                AppLogger.LogError("[GSX] Unexpected error", ex);

                GsxStatusTextBlock.Text =
                    $"GSX Profiles: Error checking profiles for {_lastSearchedIcao}";
                GsxStatusTextBlock.Visibility = Visibility.Visible;
            }
        }

        private void GsxStatus_Click(object sender, MouseButtonEventArgs e)
        {
            if (_lastGsxCount <= 0)
                return;

            AppLogger.Log($"[GSX] User clicked GSX link for ICAO: {_lastSearchedIcao}");

            Process.Start(new ProcessStartInfo
            {
                FileName = $"https://flightsim.to/others/gsx-pro/search/{_lastSearchedIcao}",
                UseShellExecute = true
            });
        }

        private void GsxStatus_MouseEnter(object sender, System.Windows.Input.MouseEventArgs e)
        {
            if (_lastGsxCount > 0)
                GsxStatusTextBlock.TextDecorations = TextDecorations.Underline;
        }

        private void GsxStatus_MouseLeave(object sender, System.Windows.Input.MouseEventArgs e)
        {
            GsxStatusTextBlock.TextDecorations = null;
        }

        // ================= WINDOW MOVE =================
        private void Header_MouseLeftButtonDown(object sender, MouseButtonEventArgs e)
        {
            if (e.ButtonState == MouseButtonState.Pressed)
                DragMove();
        }

        // ================= ACTIONS =================
        private void Exit_Click(object sender, RoutedEventArgs e)
        {
            AppLogger.Log("UI: Exit requested");
            Close();
        }

        private void History_Click(object sender, RoutedEventArgs e)
        {
            var dialog = new HistoryDialog
            {
                Owner = this
            };

            dialog.ShowDialog();
        }

        public void ShowProgress(string text)
        {
            StatusTextBlock.Text = text;
            DownloadProgressBar.Value = 0;
        }

        private async void PauseDownload_Click(object sender, RoutedEventArgs e)
        {
            await _downloadService.PauseCurrentDownloadAsync();
            AppLogger.Log("UI: Pause clicked");
        }

        private async void ResumeDownload_Click(object sender, RoutedEventArgs e)
        {
            await _downloadService.ResumeCurrentDownloadAsync();
            AppLogger.Log("UI: Resume clicked");

        }

        private async void CancelDownload_Click(object sender, RoutedEventArgs e)
        {
            await _downloadService.CancelCurrentDownloadAsync();
            AppLogger.Log("UI: Cancel clicked");
        }

        protected override async void OnClosing(CancelEventArgs e)
        {
            AppLogger.Log("MainWindow closing requested");

            if (_downloadService.IsDownloading || _downloadService.QueueCount > 0)
            {
                var dialog = new ExitDownloadDialog { Owner = this };
                dialog.ShowDialog();

                AppLogger.Log($"Exit dialog result: {dialog.Result}");

                switch (dialog.Result)
                {
                    case ExitChoice.Continue:
                        e.Cancel = true;
                        return;

                    case ExitChoice.CancelAndExit:
                        await _downloadService.CancelCurrentDownloadAsync();
                        break;

                    case ExitChoice.ExitAndResume:
                        var session = _downloadService.GetActiveSession();
                        if (session != null)
                        {
                            _historyService.AddOrUpdate(new DownloadHistoryItem
                            {
                                Icao = session.ScenarioId.Split('_')[0],
                                ScenarioName = session.ScenarioId,
                                Developer = session.ScenarioId.Split('_')[1],
                                Method = "Torrent",
                                DownloadDate = DateTime.Now,
                                PackagePath = null,
                                IsInstalled = false,
                                AutoInstallPending = true
                            });
                        }
                        break;
                }
            }

            base.OnClosing(e);

            Application.Current.Shutdown();
        }

        private void SelectFolder_Click(object sender, RoutedEventArgs e)
        {
            var dialog = new System.Windows.Forms.FolderBrowserDialog
            {
                Description = "Select new storage folder"
            };

            if (dialog.ShowDialog() == System.Windows.Forms.DialogResult.OK)
            {
                var settingsService = new SettingsService();
                var settings = settingsService.Load();

                settings.DownloadRoot = dialog.SelectedPath;
                settingsService.Save(settings);

                MessageBox.Show(
                    "Storage location updated.\nPlease restart the application.",
                    "Restart required",
                    MessageBoxButton.OK,
                    MessageBoxImage.Information
                );
            }
        }

    }
}