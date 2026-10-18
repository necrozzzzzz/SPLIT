import unittest
from compare_trace import compare,summarize_inputs
class ComparisonTests(unittest.TestCase):
 def test_aligns_early_motion_without_hiding_later_divergence(self):
  source={'tickRate':64,'trajectory':[{'tick':i,'position':[i/64*100,0,0]} for i in range(385)]}
  rows=[]
  for i in range(360):
   t=i/64
   rows.append({'elapsed_ms':str((t+1.25)*1000),'pos_x':str(t*100+(50 if t>4 else 0)),'pos_y':'0','pos_z':'0'})
  result=compare(source,rows)
  self.assertAlmostEqual(result['offsetSeconds'],1.25,places=3)
  self.assertLess(result['earlyRmsUnits'],.1)
  self.assertGreater(result['samples'][-1]['errorUnits'],49)
 def test_rollbacks_and_unavailable_values_are_explicit(self):
  rows=[{'input_read_started_ms':'1','input_last_command':str(n),'input_buffers_match':'1','button_layout':'rawEmbeddedMember'} for n in [10,11,9,12]]
  result=summarize_inputs(rows)
  self.assertEqual(result['rollbackObservations'],1)
  self.assertEqual(result['reflectedButtonRows'],0)
  self.assertEqual(result['rawButtonRows'],4)
 def test_old_csv_reports_no_input_observations(self):
  self.assertEqual(summarize_inputs([{'elapsed_ms':'1'}])['rowsWithInputAttempt'],0)
if __name__=='__main__':unittest.main()
