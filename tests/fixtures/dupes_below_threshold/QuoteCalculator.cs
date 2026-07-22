namespace Catalog;

public class QuoteCalculator
{
    public decimal EstimateTotal(decimal unitPrice, int unitCount, decimal shippingCost)
    {
        var subtotal = unitPrice * unitCount;
        var total = subtotal + shippingCost;
        return total;
    }
}
