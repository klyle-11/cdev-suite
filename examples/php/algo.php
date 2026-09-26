<?php
// cdev watch examples/php/algo.php     (untested: php not installed when written)
class ListNode {
    public function __construct(public int $val, public ?ListNode $next = null) {}
}

$insertionSort = cdev_trace(function (array $xs): array {
    for ($i = 1; $i < count($xs); $i++) {
        $j = $i;
        while ($j > 0 && $xs[$j - 1] > $xs[$j]) {
            [$xs[$j - 1], $xs[$j]] = [$xs[$j], $xs[$j - 1]];
            $j--;
            cdev_w(['xs' => $xs, 'i' => $i, 'j' => $j], 'insertionSort');
        }
    }
    return $xs;
}, 'insertionSort');

$sorted = $insertionSort([4, 2, 5, 1, 3]);
$head = null;
foreach (array_reverse($sorted) as $v) {
    $head = new ListNode($v, $head);
}
cdev_w($head, 'list');
cdev_log('sorted', $sorted);
