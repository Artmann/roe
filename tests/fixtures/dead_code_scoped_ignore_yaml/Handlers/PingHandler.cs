using System;
using System.Threading.Tasks;

namespace Messaging.Handlers;

// Matched only by the `deadCode.ignore` glob: this class is unused, but its
// finding must be dropped — while its retry-method occurrence must still
// appear in the duplicate report, because the scoped glob applies to
// dead-code alone.
internal class PingHandler
{
    public Task HandleAsync(string requestId)
    {
        return ExecuteWithRetryAsync(async () =>
        {
            Console.WriteLine($"Handling ping {requestId}");
            await Task.CompletedTask;
        }, maxAttempts: 2);
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
