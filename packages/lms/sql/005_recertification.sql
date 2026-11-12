-- Recertification (src/enrollment.rs, src/certificate.rs): a learner has one enrolment per course and
-- cycle, and quiz attempts and submissions are counted per enrolment, not per learner. Earlier
-- releases made the unique keys on (course, learner), (quiz, learner, attempt_no) and
-- (assignment, learner, submission_no); the entities now ask for the new ones and these are dropped.
drop index if exists uq_e_lms_enrollment_course_learner;
drop index if exists uq_e_lms_quiz_attempt_quiz_learner_attempt_no;
drop index if exists uq_e_lms_assignment_submission_assignment_learner_submission_no;
update e_lms_enrollment set cycle = 1 where cycle is null;
