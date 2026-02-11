using SceneryAddonsBrowser.Logging;
using SceneryAddonsBrowser.Models;
using SceneryAddonsBrowser.Services;
using SceneryAddonsBrowser.UI;
using System.ComponentModel;
using System.Windows;
using System.Windows.Input;
using System.Windows.Media;
using static System.Windows.Forms.VisualStyles.VisualStyleElement.Button;
using Brush = System.Windows.Media.Brush;
using MessageBox = System.Windows.MessageBox;


namespace SceneryAddonsBrowser
{
    public partial class MainWindow : Window
    {
        private readonly SearchService _searchService;
        private readonly DownloadService _downloadService;
        private readonly DownloadStatus _downloadStatus = new();
        public event PropertyChangedEventHandler? PropertyChanged;

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


        public MainWindow()
        {
            InitializeComponent();

            _searchService = new SearchService();
            _downloadService = new DownloadService();

            _downloadService.StateChanged += OnDownloadStateChanged;

            _downloadService.QueueChanged += UpdateQueueUi;


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
            await _downloadService.ResumeTorrentSessionAsync(
                (text, percent) =>
                    {
                        Dispatcher.Invoke(() =>
                        {
                        DownloadProgressBar.Visibility = Visibility.Visible;
                        DownloadProgressBar.Value = percent;
                        StatusTextBlock.Text = text;
                });
            });
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

            ResultsListView.ItemsSource = results;
            StatusTextBlock.Text = $"{results.Count} result(s) found.";
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
                        _downloadService.ResumeCurrentDownloadAsync();
                        break;
                }
            }

            base.OnClosing(e);
        }
    }
}
