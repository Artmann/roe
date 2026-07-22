using System;
using System.Threading.Tasks;

namespace Billing;

public class NotificationService
{
    public Task SendAsync(string recipientId, string message)
    {
        return RunWithRetryAsync(async () =>
        {
            Console.WriteLine($"Sending to {recipientId}: {message}");
            await Task.CompletedTask;
        }, maxRetries: 4);
    }

    private async Task RunWithRetryAsync(Func<Task> operation, int maxRetries)
    {
        var count = 0;

        while (true)
        {
            try
            {
                await operation();
                return;
            }
            catch (Exception ex) when (count < maxRetries)
            {
                count++;
                var wait = TimeSpan.FromSeconds(Math.Pow(2, count));
                Console.WriteLine($"Retry {count} after {ex.Message}, waiting {wait}");
                await Task.Delay(wait);
            }
        }
    }
}
