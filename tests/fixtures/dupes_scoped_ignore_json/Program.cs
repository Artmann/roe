namespace Billing;

public static class Program
{
    public static void Main()
    {
        var payments = new PaymentService();
        payments.ChargeAsync("customer-1", 25m).Wait();
    }
}
