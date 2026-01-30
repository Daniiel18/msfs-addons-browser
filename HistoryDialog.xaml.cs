using SceneryAddonsBrowser.Services;
using System.Windows;
using System.Windows.Input;

namespace SceneryAddonsBrowser
{
    public partial class HistoryDialog : Window
    {
        public HistoryDialog()
        {
            InitializeComponent();

            var service = new HistoryService();
            DataContext = service.Load();
        }

        private void Close_Click(object sender, RoutedEventArgs e)
        {
            Close();
        }

        private void Header_MouseLeftButtonDown(object sender, MouseButtonEventArgs e)
        {
            if (e.LeftButton == MouseButtonState.Pressed)
            {
                DragMove();
            }
        }
    }
}
