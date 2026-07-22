using System;
using System.Threading.Tasks;

namespace Billing.Legacy;

public class OldShippingService
{
    public Task DispatchAsync(string orderId)
    {
        return ExecuteWithRetryAsync(async () =>
        {
            Console.WriteLine($"Dispatching order {orderId}");
            await Task.CompletedTask;
        }, maxAttempts: 5);
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
