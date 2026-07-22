using System;
using System.Threading.Tasks;

namespace Billing;

public class PaymentService
{
    public Task ChargeAsync(string customerId, decimal amount)
    {
        return ExecuteWithRetryAsync(async () =>
        {
            Console.WriteLine($"Charging {customerId} for {amount:C}");
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
