using SceneryAddonsBrowser.Logging;
using SceneryAddonsBrowser.Models;
using SceneryAddonsBrowser.Services;
using System.Threading.Tasks;
using System.Windows;
using System.Windows.Input;

namespace SceneryAddonsBrowser
{
    public partial class MainWindow : Window
    {
        private readonly SearchService _searchService;
        private readonly DownloadService _downloadService;

        public MainWindow()
        {
            InitializeComponent();
            _searchService = new SearchService();
            _downloadService = new DownloadService();
        }

        // ================= AUTOFOCUS =================
        private void IcaoTextBox_Loaded(object sender, RoutedEventArgs e)
        {
            IcaoTextBox.Focus();
            Keyboard.Focus(IcaoTextBox);
        }

        // ================= ENTER = SEARCH =================
        private async void IcaoTextBox_KeyDown(object sender, KeyEventArgs e)
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

            bool? result = dialog.ShowDialog();

            if (result == true && dialog.SelectedMethod != null)
            {
                try
                {
                    StatusTextBlock.Text = $"Starting {dialog.SelectedMethod.Name}...";
                    DownloadProgressBar.Visibility = Visibility.Visible;

                    await _downloadService.StartDownloadAsync(dialog.SelectedMethod);

                    StatusTextBlock.Text = "Download started.";
                }
                catch
                {
                    StatusTextBlock.Text = "Download failed.";
                }
                finally
                {
                    DownloadProgressBar.Visibility = Visibility.Collapsed;
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
    }
}
