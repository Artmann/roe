using System;
using System.Threading.Tasks;

namespace Messaging.Services;

public class PingService
{
    public Task PingAsync(string hostId)
    {
        return ExecuteWithRetryAsync(async () =>
        {
            Console.WriteLine($"Pinging {hostId}");
            await Task.CompletedTask;
        }, maxAttempts: 3);
    }

    private async Task ExecuteWithRetryAsync(Func<Task> action, int maxAttempts)
    {
        var attempt = 0;

        while (true)
        {
            try
            {
                await action();
                return;
            }
            catch (Exception exception) when (attempt < maxAttempts)
            {
                attempt++;
                var delay = TimeSpan.FromSeconds(Math.Pow(2, attempt));
                Console.WriteLine($"Retry {attempt} after {exception.Message}, waiting {delay}");
                await Task.Delay(delay);
            }
        }
    }
}
