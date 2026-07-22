namespace Catalog;

public class PriceCalculator
{
    public decimal CalculateTotal(decimal price, int quantity, decimal shippingCost)
    {
        var subtotal = price * quantity;
        var total = subtotal + shippingCost;
        return total;
    }
}
